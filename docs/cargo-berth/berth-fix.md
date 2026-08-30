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
  - `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — Python coordinator, 2,690 lines: generated-table imports `FIXED_STATUS_EXIT_CODES` (`:22`) and `STATUS_PAYLOAD_KINDS` (`:27`), consumed in `parse_envelope` at `:879` and `:895`. `ReleaseDispositionValue` (`:129`), `ReservationLifecycleValue` (`:165`) over `ActiveReservationLifecycleValue` (`:137`) and `OutstandingReservationLifecycleValue` (`:143`), `UnknownReservationLifecycleValue` (`:181`), `ReservationLifecycleQueryStateValue` (`:189`), `_release_disposition` (`:2028`), `reservation_lifecycle_state` (`:2105`). `EnvelopePayload` (`:82`) with `alerts: list[object]` (`:86`) and `data: NotRequired[dict[str, object]]` (`:87`); `EnvelopeValue` (`:90`); `ValidatedEnvelope` (`:462`); `parse_envelope` (`:837`), whose `expected_verb: str` is at `:838` and whose verb check is at `:869`. `ProposalAwaitingApprovalStateValue` (`:366`) with `proposal: dict[str, object]` (`:374`). `ClaimTransition` (`:527`) over `NeutralClaimTransition` (`:504`), `AnsweredClaimTransition` (`:509`), and `ApprovedClaimTransition` (`:518`). `CoordinationIdentityRejectionValue` (`:604`), `RenderedCoordinationIdentityRecoveryActionValue` (`:611`) with its `kind: str` at `:614`, `ForeignActorIncursionResolutionValue` (`:660`), `ReplayFailureValue` (`:730`). Four `tagged()` erasure points: `:493`, `:626`, `:678`, `:744`. Classifiers all returning `dict[str, object]`: `classify_claim` (`:1485`), `classify_check` (`:1714`), `_validate_board` (`:1800`), `render_board` (`:1893`), `coordinator_state` (`:2190`) — Phase 13's rename of `_generic_state`, whose generic `invalid_input` branch (`:2218`) must stay below `_coordination_identity_classification` (`:2349`) and `_foreign_actor_incursion_resolution_classification` (`:2422`), each of which refuses any envelope whose status is not `invalid_input`. `_replay_failure_classification` (`:2375`) with its `REPLAY_FAILURE_SUBJECT_KINDS.get(reason)` lookup at `:2403`. `installed_engine_binary` (`:1968`); `CoordinatorArguments` (`:2465`) with `expected_verb: str = ""` at `:2470` and `proposal: str = ""` at `:2481`.
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
- The non-scaling standard is **exact argv equality**, not a sublinear trend: `distinct_cold_proof_subjects_are_bounded_to_one_git_evaluation_per_target` (`tests/board.rs:2254`) asserts 14 git argv for an equivalent cold check, 13 for a different one, the same multiset of git command names at one reservation and at twenty — compared order-independently through `canonical_git_command_sequence`, because Phase 16's concurrent scoped-patch reads make emission order nondeterministic — and zero `merge-base --is-ancestor` calls.
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

### Phase 15 — Bound the ledger projection to replayed facts  · status: done

#### As-built

The published projection carries only what is read back from it: schema version,
repository instance id, generation, journal end offset, and journal fingerprint. The
write-only `events: Vec<JournalEvent>` copy and its clone in `Projection::from_replay`
are gone, so publish cost is set by live replay state rather than by the number of
journal events ever written. Both event consumers already took their events from the
journal replay, so nothing migrated.

The projection owns `CURRENT_PROJECTION_SCHEMA_VERSION = 3`, independent of the
journal's `CURRENT_SCHEMA_VERSION`, which stays at 2; `MINIMUM_SUPPORTED_SCHEMA_VERSION`
stays at 1. `read_once` reads a small `ProjectionSchemaHeader` and validates its version
before decoding the cache's shape, mirroring the journal's own header-first read.
`read_validated` returns `ProjectionSynchronization::RebuildRequired` for three cases —
a missing file, an older schema version, and a version too new for this binary to decode
— so an unreadable cache is discarded and rebuilt rather than failing the command.
Malformed bytes, a repository-identity mismatch, a cache ahead of the journal, and a
fingerprint mismatch stay fatal, because none of them establishes that the file is a
readable cache for this repository. The journal is untouched: byte-identical records,
same replay, same order.

**Files:**
- `crates/cargo-berth/src/ledger/projection.rs` — the cache's shape, `ProjectionSchemaHeader`, the header-first read, and the rebuild routing in `read_validated`
- `crates/cargo-berth/src/ledger/constants.rs` — the projection's own schema constant beside the journal's
- `crates/cargo-berth/src/ledger/mod.rs` — the projection unit assertion
- `crates/cargo-berth/tests/ledger.rs` — size invariance across journal length, old-format acceptance, previous-version rebuild, and unsupported-version rebuild
- `crates/cargo-berth/tests/liveness.rs` — two disposition assertions read from the journal rather than from the cache file

**Binds later work:** The journal and the projection no longer share a version constant;
raising the journal's rewrites every appended record and makes the ledger unreadable to
any worktree on an older binary. An unreadable cache now rebuilds silently, and the cost
of that rebuild is a cache publication rather than a second pass over the journal, since
replay already precedes projection validation — so the timing phase asserts the on-disk
cache is at the current version before each engine sample, and exempts the row that
returns before the engine is reached. That phase also carries a journal-age dimension,
because this work bounded the cache and not the journal read in front of it.
`ProjectionSchemaHeader` and `ProjectionSynchronization` state their roles and carry no
bare `Option<T>`; the naming phase confirms them rather than renaming them.

**Gotchas:** A rebuild test driven through `init` cannot fail, because `init` publishes
unconditionally — proving a rebuild needs a mutation that consults the stale cache. A
projection size assertion must not compare raw byte length: the fingerprint, end offset,
and generation vary in decimal width run to run, and a raw comparison failed at 185
versus 186 bytes. Running a locally built binary against a shared ledger publishes the
new cache format to every worktree sharing it; `cargo-berth init --repair-projection`
rebuilds the cache from the journal without changing history.

**Ruled out:** A bounded summary of replayed facts in place of the removed field — new
surface with no consumer, when the finding was that the existing surface had no reader.
Bumping the shared `CURRENT_SCHEMA_VERSION` so an older binary could name the cause —
unreachable, because that binary decodes the whole cache before checking any version.

### Phase 16 — Prove the PostToolUse path stays inside 0.20 seconds  · status: done

#### As-built

The PostToolUse path is measured, not bounded, and the 0.20-second bound stands unwidened over a red matrix. The installed-engine matrix runs 640 ready samples: five independently restored samples per cell at cold and warmed executable-page temperatures, across 21 engine outcomes, both durable-proof states, two journal ages (12 records and 214), shallow/deep/divergent histories at one and twenty subjects, lost-evidence at one and twenty alerts, and board reads at one and fifty incidents with reservations held fixed. `NeedsRepair` is its own non-engine row. Each cold executable is mapped, invalidated with Darwin `msync(MS_INVALIDATE)`, and required to report zero resident pages from `mincore` before the clock starts; a cell that will not go cold fails rather than being relabeled warm. 273 of 320 cold and 244 of 320 warm samples land at or above 0.20 s — largest cold `first_touch_acquisition` 0.590941 s with five Git argv, largest warm lost-evidence resolved-trunk at one alert 0.531557 s with 20 Git argv, largest Git count 22 in uncached `successor_equivalence_positive`. Cost and correctness changes that landed alongside the measurement: the working-tree read is one concurrent `status --porcelain -z --no-renames --untracked-files=all`, `Ledger::open_from_discovered_worktree` serves the engine path, the four scoped-patch reads run concurrently and unconditionally, the `-Xno-renames` strategy is selective and gated on `ProtectedScopedRename`, and `scoped_merge_displaced_reserved_file` matches Git's synthetic displaced path exactly.

**Files:**
- `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — the ready-engine timing matrix, the cold-residency gate, and `PostToolUseTimingCell`; the only place the full shim path executes
- `crates/cargo-berth/src/drift/observation.rs` — the single concurrent working-tree read run against the phase-history walk
- `crates/cargo-berth/src/git/mod.rs` — concurrent scoped-patch reads, the `ProtectedScopedRename` gate on `-Xno-renames`, and `scoped_merge_displaced_reserved_file`
- `crates/cargo-berth/tests/board.rs:3870`, `crates/cargo-berth/tests/edges.rs:830` — `canonical_git_command_sequence`, the order-independent half of the cardinality guards

**Binds later work:** Wall time is not driven by Git process count: fitting the 57 cold per-cell maxima against Git argv gives ~8.6 ms per Git process over a ~0.234 s intercept at zero Git processes, so the floor alone exceeds the budget and the five-Git outcome is the slowest cell while the 22-Git outcome is not. Any plan reaching 0.20 s by cutting Git arity is unreachable on its own arithmetic — the phase that decomposes and bounds the PostToolUse fixed cost owns attributing that 0.234 s floor, and re-measures both temperatures. The concurrent working-tree read, the selective `-Xno-renames` gate, and the exact displaced-path match are correctness, not speed, and must not be reverted for speed. Nothing asserts an absolute ceiling on Git processes per outcome — cardinality invariance is asserted, magnitude is not, which is why a 22-process path could ship; a per-outcome ceiling belongs beside `PostToolUseTimingCell` in the Python harness, not in `post_tool_use_git_subprocess_count_is_cardinality_invariant`, which drives berth's installed Git post-commit hook rather than the external shim.

**Gotchas:**
- A cost exists that no argv guard can see: `history[profile=deep]` spends nine Git argv at both cardinalities yet costs 0.249653 s at one subject and 0.418663 s at twenty. That 0.17 s is per-subject in-process work, and cardinality assertions compare argv counts that are equal here by construction.
- The scheduling-switch trap: `scoped_patch_read_scheduling()` returned `SerializedForInstrumentedGit` whenever `CARGO_BERTH_TEST_REAL_GIT` was set, and the harness sets it on every measured invocation, so every timed scoped-patch sample ran serialized. The type is deleted. Standing rule: a harness must never set an environment variable the engine reads as a scheduling switch — a measurement that changes the code path it measures is void however green it reads.
- The cardinality guards compare through `canonical_git_command_sequence`, which sorts before comparing, because concurrent scoped reads make trace emission order nondeterministic. This is not a weakened oracle: exact argv totals stay asserted separately and are what catch a per-reservation call returning. Do not restore an order-sensitive comparison.
- Cold executable pages can be evicted unprivileged on this host — `mmap` + `msync(MS_INVALIDATE)` + `mincore` took the installed engine from 136 of 300 resident pages to 0. Cold and warm differ far less than expected; page residency is not the lever.

**Ruled out:**
- Widening the bound, adding a tolerance, skipping a row, or excluding a cell to obtain a green matrix — more red cells is the correct result of measuring more dimensions.
- Caching raw Git reads to buy the bound — a memoized answer a concurrent worktree can invalidate is a correctness regression; the target-bounded retained-verdict set in `reservation/mod.rs` is not a precedent.
- Deriving `AheadBehind::Unrelated` from `rev-list --left-right --count` — it reports symmetric-difference counts and says nothing about merge-base existence.
- Hoisting reads out of `compare_scoped_patch`'s per-target loop — `phase_start_head` arrives already resolved and the budgets admit one comparison per target per pass.
- Further parallelism — concurrency bought the first order of magnitude (worst case 7.343 s to 0.281737 s) and then stopped helping; the remainder is per-process spawn overhead no overlap removes.

### Phase 17 — Decompose and bound the PostToolUse fixed cost  · status: done

#### As-built

The PostToolUse fixed cost is decomposed into a seven-component ladder measured on the
production-faithful harness at both process-cache temperatures, and the measurement establishes that
the decomposition cannot be validated at five samples: two back-to-back runs against one
fingerprint-locked binary produce 7.24% and 13.97% error against the same warm intercept, six of
fourteen component/temperature pairs fail the spread test, and negative components persist on a
stable binary. The 0.20-second bound remains unmet and visible — eleven cold and three warm samples
exceed it, at maxima of 0.492390s and 0.577176s.

Three defects surfaced by that measurement are repaired. `git::reference_lookup` resolves through
`show-ref --exists` and returns `Missing` only on exit 2; a failed `rev-parse`, a spawn failure, and
malformed output each propagate as `GitError`, where previously any non-zero exit read as absence.
`ClaimRepositoryFacts::read` goes straight to the live read, dropping a filesystem HEAD read whose
result was discarded; `requires_live_head_revalidation` remains called and reachable.
`ClaimError::GitBackedReferenceRequired` is deleted — the reftable signal is a private
`ReferenceFileReadError::GitResolutionRequired` inside the filesystem reference reader, and a genuine
Git failure still reaches the caller as `ClaimError::Git(GitError)`.

`WorktreeContext::from_registered_root` canonicalizes the registered root once before joining `.git`
and returns `RegisteredWorktreeAvailability::Unavailable` when canonicalization fails, so a root
reached through a symlink now yields the same repository root `discover` produces.

The Python timing harness accumulates all seven per-sample expectations and the hook-executable
expectation with expected value, observed value, and raw sample argv, entering `matrix_summary` and
failing the run afterwards rather than aborting on the first. It never installs: `setUpClass` refuses
to run when `CARGO_BERTH_TIMING_REPOSITORY_ROOT` is set and names the explicit
`--install-engine /path/to/cargo-liner` command, and `installed_timing_artifact_digests()` takes a
sha256 of the engine and both generated consumers and rejects a measurement if any changes mid-run.

**Files:**
- `crates/cargo-berth/src/git/mod.rs` — `reference_lookup` at `:1703`, the `show-ref --exists` resolution
- `crates/cargo-berth/src/verb/claim.rs` — `ClaimRepositoryFacts::read` at `:1228`; the private `ReferenceFileReadError::GitResolutionRequired`
- `crates/cargo-berth/src/ledger/mod.rs` — `WorktreeContext::from_registered_root` at `:379`, canonicalizing before the `.git` join
- `crates/cargo-berth/src/worktree/liveness.rs` — `registered_worktree_location` at `:321`, the sole caller; the canonicalization test at `:426`
- `crates/cargo-berth/tests/lifecycle.rs` — four reference-resolution regression tests from `:452`
- `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — the seven-component ladder, accumulated expectations, digest guard, and no-install `setUpClass`; lives in the `/Users/natemccoy/.claude` checkout and never joins a `cargo-berth` commit

**Binds later work:** `reference_lookup` returns `Missing` only on Git's own not-found exit; collapsing
any other exit back into absence reintroduces the repaired defect. The reftable signal stays private —
no wire status, payload member, or user-facing diagnostic names it, and the deleted claim error is not
to be reintroduced under any name. `from_registered_root` canonicalizes and reports `Unavailable`; no
test reaches that alternative yet, and the semantic-roles phase owns closing that gap. The timing
harness never self-installs, and its exit-20/21/22 protocol, digest guard, and unresolved-attribution
reporting are preserved by whatever edits that file next.

**Gotchas:** The attribution's ten-percent gate cannot be met at five samples, however many rungs the
ladder gains — the same binary passes and fails it on consecutive runs. `~/.claude` is a separate Git
checkout, so its half of any phase is invisible to a `cargo-berth` diff and must be read by absolute
path. A failing test reported as pre-existing by a delegate is not: `git show HEAD:<path>` settles it,
and here it proved a test added by this phase that had never passed.

**Ruled out:** Widening the ten-percent attribution gate, or adding ladder rungs to reach it — an
unprovable attribution is a finding to report with its evidence. Defining the shim-invocation term as
the production route minus two of its own components — the remaining five telescope to exactly those
subtracted terms, so the sum is identically the production route and the check cannot fail. Reverting
the accidental install of this worktree's engine — it is newer than what it replaced, and rebuilding
to restore the prior state buys nothing.

### Phase 18 — Semantic roles and bounded optionality  · status: done

#### As-built

Twenty types name their semantic role rather than their representation, and no domain-state `Option<T>` survives at the boundaries that carried one. The new semantic types:

- `CommandOutputOwnership::{CallerRendersResponse(Box<OutputEnvelope>), BoardPresentedAndTerminalRestored}` (`cli.rs`) — `Cli::run` is its only observer.
- `OverlapSelection::{NoOverlapRequested, RequesterBeforeHolder, RequesterAfterHolder, Defer, Override}`; each permissive variant carries `blocker_reservation_id`, `authorization_reason`, and `proposal_submission`. `overlap_selection(before, after, defer, override_reservation, overlap_why: Option<&str>, proposal: Option<&str>) -> Result<OverlapSelection, String>` converts all six clap optionals in one place, called once by `into_claim_request` with `?`, so no helper receives a raw optional. One match over the four reservation optionals ends in a reachable `_ => Err("choose only one overlap answer")`.
- `HarnessSessionId` is `pub(crate)` and travels as itself through `PostToolUseDriftInvocation`, its `OnceLock<HarnessSessionId>`, and `select_current_process_harness_session(harness_session_id: HarnessSessionId)`; `from_current_process` does not re-parse the stored value.
- `EnvironmentCoordinationRunSelection::{NotSupplied, UnusableFallbackToMarker, Identified(CoordinationRunId)}` — internal to `EditAuthorization::resolve_from_sources` and converted before its single authorization read, preserving the one-read guarantee.
- `WorktreeComparability::{Comparable(WorktreeId), IdentityUnavailable, DeferredPendingRewrite}` replaces `Result<Option<WorktreeId>>`; the sole consumer normalizes both non-comparable states to the same unchanged, empty drift report.
- `FirstTouchHolderRecoveryDescription::{NotApplicable, Available(String)}` — the string is the recovery description naming the verbs that clear a holder; emitted blocked-message text is byte-identical.
- `FilesystemReferenceResolution::{Resolved(GitObjectId), RequiresGitResolution { rejection_if_git_reports_missing }}`, private to the filesystem reference reader. The producer picks the fallback error, so `read_reference` matches two arms and never inspects a payload; a genuine Git failure still reaches the caller as `ClaimError::Git`, and no wire status, payload member, or diagnostic names the reftable fallback.
- `GitCommandOutputAvailability::{Available(Output), Unavailable(io::Error)}` — the carried `io::Error` and its exact diagnostic survive the conversion.
- `PostCommitRecoveryMarkerAction::{NoMarkerPublicationRequired, PublishMarker(CoordinationRunId)}`.
- `BoardReservationSnapshot` combines journal-derived lifecycle with computed `edit_blocking_status`, integration evidence, visibility, freshness, and live `ahead_behind_main`; the blocking status is read from `Reservation`'s computed method, never stored.

**Files:**
- `crates/cargo-berth/src/cli.rs` - `CommandOutputOwnership`, `OverlapSelection`, `overlap_selection`, and the PostToolUse payload parser (`:1107`); `session/mod.rs` - `HarnessSessionId`, `select_current_process_harness_session`
- `crates/cargo-berth/src/ledger/mod.rs` - `EnvironmentCoordinationRunSelection`; `verb/claim.rs` - `FilesystemReferenceResolution`, private to the reference reader; `board/mod.rs` - `BoardReservationSnapshot` (`:145`); `output.rs` - `FirstTouchHolderRecoveryDescription`
- `crates/cargo-berth/src/git/mod.rs`, `git/command.rs` - `GitCommandOutputAvailability`, `ProtectedTipSuccessorHeadClassification`, `ScopedReplayRenameDetection::{DisabledWithoutProtectedRename, RequiredForProtectedRename}`, `LocalBranchRenameTargetResolution::{NotProven, Unique(FullRefName), Ambiguous}`
- `crates/cargo-berth/src/drift/execution.rs`, `drift/classification.rs`, `drift/git_output.rs` - `WorktreeComparability`, `PreLockForeignPathClassification`, `WorkingTreeChangePartition`
- `crates/cargo-berth/src/reservation/mod.rs`, `reconcile.rs` - `RetainedScopedPatchTargetVerdict(s)`, `ScopedPatchTargetVerdictAvailability`, `ScopedPatchTargetEvaluationSchedule`, `ReconciliationScopedPatchEvaluationBudget`, and the four matching successor twins
- `crates/cargo-berth/src/recovery.rs`, `alert.rs` - `PostCommitRecoveryMarkerAction`, `LostEvidenceRecoveryCommand`

**Binds later work:** The PostToolUse payload parser now parses `session_id` into `HarnessSessionId` and returns `PostToolUseDriftInvocationError::InvalidPayload` for an overlong or control-character value, where the previous emptiness-only filter accepted both. Its Rust proof belongs to *Acceptance proof for the Phase 18 semantic states*; the hook message at `berth_post_bash.sh:421` still states only the non-empty rule and belongs to *Coordinator tagged unions and hook rendering*. `HarnessSessionId` arrives typed at every consumer, so none re-validates it. `FilesystemReferenceResolution::RequiresGitResolution` is the name downstream constraints key on; `ReferenceFileReadError` no longer exists.

**Gotchas:**
- `HarnessSessionId`'s contract is 1 to 256 characters with no control characters. Re-validating an already-typed id duplicates a guarantee the type carries.
- `git::reference_lookup` returns `Missing` only on exit 2; every other outcome propagates as `GitError`. `RequiresGitResolution`'s fallback error names what git reports missing, not that git is empty.
- The board's TUI path restores the terminal itself. Rendering `CommandOutputOwnership::BoardPresentedAndTerminalRestored` as output double-presents.
- This package's suite contains wall-clock-bounded tests. Concurrent verification runs push them past their deadlines even when compiles are serialized; those failures are contention and clear on a quiet rerun.

**Ruled out:**
- A three-state `FilesystemReferenceResolution` with `Rejected(ClaimError)` — the caller would destructure it to pick a fallback error.
- Owned `Option<String>` for `overlap_why` and `proposal` on the boundary function — four match arms read them through `.as_deref()` and never consume them, so they are `Option<&str>`.
- Inlining the overlap conversion into `into_claim_request` — it exceeds the line lint and lets raw clap optionals reach post-conversion helpers.
- The names `BoardReservationState` (claims an authority a computed blocking decision does not give it), `EnvironmentRunSelection` (reads as a selection among environments), and `Absent`/`Before`/`After` (representation, and a direction without saying whose).
- Renaming `DeferredScopedPatchIntegrationStatus` or `ScopedPatchEvaluationPriority` — both already state their role.

### Phase 19 — Acceptance proof for the Phase 18 semantic states  · status: done

#### As-built

Five acceptance tests construct the semantic states Phase 18 introduced and assert what a caller observes in each. `unusable_environment_coordination_run_falls_back_to_marker_then_unidentified` proves `EnvironmentCoordinationRunSelection::UnusableFallbackToMarker` resolves to the worktree marker when one exists and to unidentified when none does, rather than being treated as absent or accepted. `drift_reports_no_change_when_worktree_identity_is_unavailable` and `drift_reports_no_change_while_a_rewrite_is_pending` each assert the same unchanged, empty drift report, so widening one branch without the other fails. `overlong_harness_session_id_is_an_invalid_payload` and `control_character_harness_session_id_is_an_invalid_payload` each assert `PostToolUseDriftInvocationError::InvalidPayload` at the PostToolUse payload boundary.

Two of those states were unreachable through the public path and were repaired in the production code rather than reached through a test seam. `prepare_drift_execution` now branches on the reservation selection: the post-commit arm determines worktree comparability *before* creating identity, so `IdentityNotRecorded` is reachable at all. `PreparedDriftExecution::CompletedUnchanged(Box<DriftReport>)` replaces the former `NothingToCompare { comparison }` variant, and a `WorktreeComparisonReadiness` enum — `Ready(WorktreeId)` or `CompletedUnchanged(Box<DriftReport>)` — carries the decision out of `prepare_worktree_comparison`; the old `nothing_to_compare` helper is gone.

`comparable_worktree` no longer turns every identity-read failure into a clean "unchanged" report. It stands aside for exactly one case — the identity file is genuinely absent (`LedgerError::Io` with `ErrorKind::NotFound`) under `DriftReservationSelection::EveryActiveForPostCommit` — and propagates every other ledger error as a `DriftExecutionError`. A malformed worktree identity now fails loudly instead of reading as no drift. The state is named `IdentityNotRecorded` rather than `IdentityUnavailable`, because unavailability is what the old swallow claimed and absence is what it actually means.

**Files:**
- `crates/cargo-berth/src/drift/execution.rs` — `WorktreeComparability`, `WorktreeComparisonReadiness`, `prepare_worktree_comparison`, the selection-branched `prepare_drift_execution`, and the unit tests for both stand-aside states
- `crates/cargo-berth/src/ledger/mod.rs` — `EnvironmentCoordinationRunSelection` and `EditAuthorization::resolve_from_sources`, with the extended precedence test
- `crates/cargo-berth/src/cli.rs` — the PostToolUse payload parser producing `HarnessSessionId`, with both rejection tests
- `crates/cargo-berth/src/session/mod.rs` — `HarnessSessionId`'s parse contract
- `crates/cargo-berth/tests/drift.rs` — the integration regression proving an unreadable identity does not report a clean drift

**Binds later work:** `HarnessSessionId` accepts 1 to 256 **characters**, not bytes — a 256-character multibyte id is valid — and rejects any control character; the hook message stating that contract is still the pre-Phase-18 non-empty wording at `scripts/berth/install/hooks/berth_post_bash.sh:422`. `WorktreeComparability::IdentityNotRecorded` is the name to key on. `docs/cargo-berth/generated/output-contract.json` is rewritten only by `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1` inside the reproducibility test, and it carries the embedded Python module the installer derives.

**Gotchas:** The `run_hook` fixture in `scripts/berth/tests/test_hook_rendering.py` returns its configured engine exit regardless of the payload handed to it, so a hook rejection-message test written against it passes without ever reaching the payload parser; proving a rejection message requires a real installed-engine invocation. Two reservations can be simultaneously active for one worktree and coordination run: `partition_first_touch_protected_scopes` picks the first covering one in replay order and republishes that id into the session mapping, so a fresh claim can be shadowed by a stale reservation and widening then lands on the wrong one.

**Ruled out:** Releasing a stale reservation to get past the two-active condition — it is an irreversible append to a ledger three live worktrees share, and it treats the symptom rather than the selection defect. Reaching the unconstructible states through a test-only seam instead of repairing the production path — the states were unreachable because the code was wrong, not because the test lacked access.

### Phase 20 — Generated semantic discriminators for the Python consumer  · status: done

#### As-built

The Rust contract emitter generates eleven semantic `Literal` aliases into `scripts/berth/generated/status_payload_tables.py`: `CommandVerbValue`, `OutputStatusValue`, `OutputPayloadKindValue`, `IntegrationProofValue`, `TrunkObservationAtClaimValue`, `LostIntegrationEvidenceStatusValue`, `ReplayFailureReasonValue`, `ReplayFailureSubjectKindValue`, `CoordinationIdentityRejectionKindValue`, `PayloadDataRequirement`, and `OutcomeEmissionDisposition`. Alongside them it generates four boundary guards — `is_command_verb_value`, `is_output_status_value`, `is_output_payload_kind_value`, and `is_replay_failure_reason_value` — each taking `object` and returning `TypeGuard[...]` by testing membership in the corresponding generated set, so a decoded JSON value narrows once at the parse boundary and stays narrowed.

The aliases are consumed inside the generated module rather than merely exported. `OutcomeRule` is a `NamedTuple` whose `verb`, `status`, `payload_kind`, `data_policy`, and `emission` fields carry semantic types, the tables are keyed and valued on those aliases, and `valid_outcome_tuple` takes them as parameters. No `frozenset[str]` or `dict[str, str]` remains. `COORDINATION_IDENTITY_REJECTION_KINDS` is derived from the same contract pass that produces the outcome rules, and `_valid_identity_rejection` reads that generated set instead of a restated literal list, so the generated validator and the rules cannot drift.

`renaming_a_rust_type_keeps_generated_artifacts_byte_identical` declares a genuinely distinct enum carrying the same `schemars` wire name and asserts the artifacts are byte-identical. It previously aliased a type to itself, which made the comparison a tautology and proved nothing about wire-name pinning.

Six domains stay `str` by construction: `OutcomeRule.discriminants` path elements and values, `required_paths` and `forbidden_paths` elements, `_value_at`'s path, and `Mapping[str, object]` keys are arbitrary JSON member names and free-form path domains with no closed set to generate from.

**Files:**
- `crates/cargo-berth/src/output_contract.rs` — the contract emitter and its generation tests
- `docs/cargo-berth/generated/output-contract.json` — the checked contract artifact, carrying the embedded Python module the installer derives
- `docs/cargo-berth/json-contract.md` — describes the exported aliases and guards
- `/Users/natemccoy/.claude/scripts/berth/generated/status_payload_tables.py` — the generated consumer module, in a separate Git checkout that no `cargo-berth` commit can include

**Binds later work:** the coordinator adopts these types rather than hand-writing them; a literal set copied into `claim_state.py` is the drift this generation exists to prevent. Nine aliases are coordinator-facing — the three envelope discriminants, the two replay domains, the identity-rejection kinds, the lost-evidence status, and the two board-rendering domains — while `PayloadDataRequirement` and `OutcomeEmissionDisposition` serve only the generated validator. Tightening the tables left one inherited diagnostic in the coordinator: `REPLAY_FAILURE_SUBJECT_KINDS.get(reason)` at `claim_state.py:2403` requires `ReplayFailureReasonValue` where `reason` is still `str`, so the coordinator's type-check baseline is red until narrowing through `is_replay_failure_reason_value` lands.

**Gotchas:** regeneration runs inside the test suite, not as a separate binary — `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1 bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` rewrites the checked contract from `generated_artifacts_are_reproducible_from_the_checked_in_contract`, and a second run leaves every artifact byte-identical. The generated module and the Rust emitter live in different Git checkouts, so a single commit can never carry both halves of a regeneration.

**Ruled out:** hand-copying `Literal` sets into `claim_state.py`, which creates a second unchecked copy of the closed sets; and narrowing the open `str` domains above, whose values are arbitrary JSON member names.

### Phase 21 — Generated-domain adoption and the semantic envelope boundary  · status: done

#### As-built

Every coordinator-facing closed set comes from `berth.generated.status_payload_tables` — nine generated aliases and four guards imported at the top of `claim_state.py`, with no hand-copied literal set anywhere in the coordinator.

The envelope boundary is two values rather than one. `ValidatedWireEnvelopeValue`, with `ValidatedWireEnvelopePayloadValue`, echoes the payload exactly as received and keeps alerts as raw `list[dict[str, object]]`; `CoordinatorEnvelopeReadingValue` carries what was read — a typed `EnvelopeAlertValue` list and a `CoordinatorPayloadReadingValue`. `ValidatedEnvelope` owns both, as `value` and `reading`.

`EnvelopeAlertValue` is a two-member tagged union — `lost_integration_evidence` and `orphaned_outstanding` — built by narrowing on `kind` and reconstructing each member's fields. `BlockedCheckPayloadReadingValue` and `BlockedClaimPayloadReadingValue` are separate types, with `BlockedEditPayloadReadingValue` as their union alias. `NoPayloadFactsValue` and `DeliberatelyUninspectedPayloadValue` are the two absence readings, and neither exposes a raw mapping.

Argparse converts every operation into a `CoordinatorRequest` member at exactly one site. A claim proposal whose answer or authorization reason contradicts the displayed text raises `EnvelopeValidationError` at parse time, before any approval can apply.

`HookTimingTests` is its own separately invocable class, split out of `HookRenderingTests`, and `run_installed_engine_hook` drives the installed engine rather than a stubbed exit.

**Files:**
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — the generated-alias imports, the split wire and reading values, the alert union, the payload reading union, and the request conversion
- `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — the boundary fixtures, the separately invocable `HookTimingTests`, and `run_installed_engine_hook`

**Binds later work:** `ValidatedEnvelope` owns `value` and `reading`; `EnvelopePayload` no longer exists. The wire value echoes raw alert mappings and the reading carries typed `EnvelopeAlertValue` members — consume the typed side and never re-derive an alert from the wire value. `BlockedEditPayloadReadingValue` is the union alias over its check and claim members. `CoordinatorRequest` is the one argument-conversion boundary. `HookTimingTests` already exists and must not be split again. The board-rendering field union and user-action type ship here as the conversion seam `render_board` consumes.

**Gotchas:** `cast(X, cast(object, y))` defeats the type checker completely, and no gate in this phase can see it — two independent reviews were what caught it. Typing the wire echo's alerts with the semantic union silently drops any member the typed alerts do not name, so the echo must keep raw mappings. These files live in the `/Users/natemccoy/.claude` checkout and can never join a `cargo-berth` checkpoint commit: diff against a pre-edit snapshot, not `HEAD`, and run tooling with `/Users/natemccoy/.claude` as the working directory so its `pyrightconfig.json` applies.

**Ruled out:** one `BlockedEditPayloadReadingValue` type covering both check and claim — it admitted states neither operation can produce. A residual payload reading carrying the raw mapping — it reintroduces the untyped surface this phase removed. Re-labelling a validated alert instead of reconstructing it.

### Phase 22 — Typed alert ownership and the invocation-local refusals  · status: todo

#### Work Order

**Goal:** Every envelope alert the preceding phase typed reaches a named production owner rather than being reconstructed from a raw mapping at each hook, and the coordinator's invocation-local refusals are proved where a user meets them — at the command line — rather than only at the exception that raises them.

**Spec:**

**No production Python code reads `ValidatedEnvelope.alerts` today.** The preceding phase gave the alert union typed members and proved that every member parses, but `test_every_envelope_alert_member_reaches_a_typed_caller` proves parsing and nothing beyond it. The three hooks each solve alert rendering independently: `berth_post_bash.sh` renders lost-integration-evidence alerts, `berth_session_start.sh` renders lost-integration-evidence alerts and board-projected orphan alerts, and `berth_pre_edit.sh` renders no alerts at all. So a typed alert exists and no typed consumer does.

Close that by naming the owner explicitly, one of two ways per alert member, and stating which was chosen in the type's own docstring:

- **Coordinator-owned.** The alert is carried through the coordinator outcome or the board outcome that the hook already consumes, so the hook reads a typed field rather than re-deriving one. This is the answer wherever the hook's rendering decision depends on a fact the coordinator already computed.
- **Hook-owned, deliberately.** The alert is rendered by a shell hook and the Python coordinator has no stake in it. This remains legitimate, but it stops being an accident: the alert type says so in a docstring, and a test asserts the coordinator does not read it, so a later reader cannot mistake the silence for an oversight.

Neither answer may leave a member unassigned. An alert member with no stated owner is the defect this phase exists to remove.

**The recovery commands are where alerts actually originate, and none of them is covered.** `resolve --integrated-as`, resolve-trunk-first, `resolve --recovered`, and the retire-or-abandon commands each drive the engine into a state that emits alerts, and the Python suite exercises none of them end to end. Each gets a fixture that runs the command's envelope through the coordinator and asserts the typed alert its owner receives.

**A contradictory claim proposal must be refused where a user meets it.** The preceding phase made a proposal whose answer or authorization reason disagrees with what was displayed raise `EnvelopeValidationError`, and its test asserts exactly that exception. But the refusal only matters because it reaches `main` and becomes exit 64 — the path a user actually travels — and no test crosses that boundary. Add a command-line assertion that a contradictory proposal cannot be approved: invoke the coordinator entry point with the contradictory proposal and assert the process refuses with exit 64 and the approval does not take effect. The refusal is invocation-local, so no ledger state may change as a result.

**`NoPayloadFactsValue` has no surface and that is the property to prove.** It is the reading for a validated envelope that explicitly carries no payload facts, and it deliberately declares no fields. Add a fixture that asserts its field surface is empty, so a later phase that quietly adds a field to it — and thereby turns "no facts" into "some facts nobody reads" — fails here rather than shipping.

Project rules that bind here: never use file-level type ignores; avoid `Any`; line-level `# pyright: ignore[reportAny]` is a last resort on the specific line only.

**Files:**
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — the alert ownership decision, the coordinator or board fields that carry typed alerts, and the docstring contract on every hook-owned member
- `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — the four recovery-command fixtures, the command-line proposal refusal, the empty-surface assertion, and the coordinator-does-not-read assertions for hook-owned members
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — read-only input: the lost-integration-evidence rendering this phase must not duplicate
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_session_start.sh` — read-only input: the lost-evidence and board-projected orphan rendering
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_pre_edit.sh` — read-only input: the hook that renders no alerts, which is itself a fact this phase records
- `/Users/natemccoy/.claude/scripts/berth/generated/status_payload_tables.py` — read-only input: the generated closed sets
- `/Users/natemccoy/.claude/commands/sync.md` — read-only input: a coordinator consumer whose expectations bound what an alert may become
- `/Users/natemccoy/.claude/commands/plan/delegate.md` — read-only input: the other coordinator consumer

**Constraints from prior phases:** The preceding phase typed the envelope boundary and split it in two — the wire value echoes the payload exactly as received, and a separate semantic reading carries what was read. Alerts live on both sides in different forms: raw on the wire value, typed on the reading. **Consume the typed side; never re-derive an alert from the wire value's raw mapping**, and never widen the wire value to carry typed members. `EnvelopePayload` no longer exists — `ValidatedEnvelope` owns `value` and `reading`, and those are the two names to use. `BlockedEditPayloadReadingValue` remains the union alias over its check and claim members. The four Phase 17 harness guards in `test_hook_rendering.py` are preserved rather than rediscovered, and `HookTimingTests` already exists as its own separately invocable class — do not split it again. Phase 20's generated module is the only source for a closed set; a hand-copied literal set is the defect Phase 14 exists to prevent. The files in this phase cannot join a `cargo-berth` checkpoint commit; establish a pre-edit snapshot of them and diff against it rather than against `HEAD`, and run tooling from `/Users/natemccoy/.claude` so its `pyrightconfig.json` applies.

**Acceptance gate:** `basedpyright scripts/berth/claim_state.py scripts/berth/tests/test_hook_rendering.py scripts/berth/generated/status_payload_tables.py`, run with `/Users/natemccoy/.claude` as the working directory, reports **zero errors, zero warnings, and zero notes** for all three files. `python3 /Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py HookRenderingTests` passes in full.

Every member of the envelope alert union has a stated owner: a test enumerates the union and fails when a member is neither reachable from a coordinator or board outcome field nor carries the hook-owned docstring contract paired with an assertion that no coordinator code reads it. Adding a member without an owner fails this gate.

Four fixtures — one each for `resolve --integrated-as`, resolve-trunk-first, `resolve --recovered`, and retire-or-abandon — drive their real envelopes through the coordinator and assert the typed alert each owner receives, reading no raw mapping to do so.

A contradictory claim proposal invoked through the coordinator entry point exits 64, the approval does not take effect, and no ledger state changes. `NoPayloadFactsValue` is asserted to declare no fields, so adding one fails here.

No file-level type ignore exists in any gated file, and every remaining `# pyright: ignore` is line-level with a named rule.

### Phase 23 — Coordinator classifier unions, board rendering, and hook text  · status: todo

#### Work Order

**Goal:** The Python coordinator's classifiers return tagged unions instead of `dict[str, object]` reached through `cast`, the board renders from typed values without changing its published JSON, and the hook states the session-id contract the engine now enforces.

**Spec:**

`classify_claim` (`claim_state.py:2250`), `classify_check` (`:2443`), `_validate_board` (`:2519`), `render_board` (`:2573`), and `coordinator_state` (`:2854`) all return `dict[str, object]`, and **29 `cast` calls** remain in the file — ten in the double-cast classifier returns, nine in board traversal, four at dispatch, and six at parsing and literal boundaries. Every tagged union the coordinator builds is erased at the one boundary a reader inspects. The weakness predates the coordinator work — `classify_claim` returned `dict[str, Any]` at `831e34a` — and repairing only one symbol would leave an inconsistent surface. Not every classifier reaches `dict[str, object]` through `cast`; all five reach it, and the return annotation is the defect regardless of how it is reached.

Name the returns rather than leaving the implementer to invent them: `classify_claim` returns `ClaimClassificationValue`, `classify_check` returns `CheckClassificationValue`, `_validate_board` returns `BoardValidationValue`, `render_board` returns `RenderedBoardValue`, and `coordinator_state` returns **`CoordinatorEngineOutcomeValue`** — the coordinator's reading of the outcome of one engine invocation. It is deliberately not `EngineInvocationStateValue`: it excludes argv, exit status, and stderr, so a name promising the invocation's state would overpromise. Name the single-attempt invocation record `SingleAttemptEngineInvocationValue` and the typed replay hard stop `ReplayHardStopStateValue`, and give the foreign-resolution outcome its own named state rather than folding it into a neighbour. **`coordinator_state` is the only classifier to retype here, not two.** Phase 13 renamed `_generic_state` to the public `coordinator_state`; there is no longer a private twin, and any instruction to retype both names a symbol that no longer exists. Phase 13 also made `installed_engine_binary` (`:2648`) public; `installed_engine_binary() -> str` is already precisely typed and needs no work here.

**Every `tagged()` conversion is an erasure point and all of them close here.** A tagged type erased at its own conversion is the same defect as an untyped classifier, so each gets an explicit typed member in every affected return union rather than a `dict[str, object]`: `EngineInvocation.tagged()` (`:693`), `CoordinationIdentityRejection.tagged()` (`:826`), `ForeignActorIncursionResolution.tagged()` (`:878`), and `ReplayFailure.tagged()` (`:944`) — the last as an explicit `ReplayHardStopStateValue` member.

**The busy-state mutation must not be smuggled into the new unions.** `main` adds `command_to_rerun` to an already-classified contention state after classification returns (`:3893` and `:3953`), so a union member written to match today's flow would need an optional field that is present in one caller and absent in another — the precise shape the type contract forbids. Introduce **two distinct types**: the contention state as classified, and the retry-ready contention state that carries the command to rerun; or construct the complete state before returning it, so no post-return mutation exists. `NotRequired`, `None`, and `dict[str, object]` are forbidden as the workaround here. Raw mappings remain permitted only at the parsing and emission boundaries.

**`render_board` must not silently change the coordinator's published contract.** The preceding phases produced `BoardRenderingValue` carrying typed fields, paths, and `{path, rendered_json}` actions, while `render_board` today traverses the wire envelope (`:2573`) and returns `{path, value}` actions. Naming its return `RenderedBoardValue` while it still reads an untyped mapping to build that value moves the erasure inward rather than removing it — so `render_board` takes the ready typed board value as its input and reads no raw mapping. But the conversion is a refactor, not a redesign: the emitted `cargo-berth-claim-state/v1` JSON must stay equivalent — same markdown order, same paths, same action values — and a parity fixture proves it against the pre-change output rather than trusting the reading of the code.

**One tag Phase 12 left open closes here.** `RenderedCoordinationIdentityRecoveryActionValue` (`:811`) types its `kind` as `str` (`:814`) where the four recovery-action tags are closed literals. That was implementation-only while `tagged()` erased the whole value to `dict[str, object]` — nothing downstream could match on it. Removing that erasure turns the tag into externally typed domain state, so it becomes a rendered union with **one member per recovery action**, and likewise one rendered type per identity rejection, rather than a single type with a widened tag.

**The invalid-input reading is an exhaustive match, and it has two distinct absence states, not one.** It covers Phase 11's three identity rejections, Phase 7's `already_recorded_by_different_coordination_actor`, **Phase 13's `ReservationLifecycleQueryRejection::UnknownReservation` as its own `UnknownReservationLifecycleQueryRejectionValue` member** (`payload.kind = reservation`, `data.status = unknown_reservation`), and a residual member. **That member is a wire-rejection reading and must stay distinguishable from the normalized coordinator state Phase 13 already ships.** `UnknownReservationLifecycleValue` (`:330`) is the *normalized* form the `reservation` operation returns, keyed `kind = unknown_reservation`; the envelope union's member is the *wire* rejection. They carry the same fact in two roles. Keep Phase 13's normalized type under its existing name, give the wire member the query-specific rejection name, and convert between them at exactly one explicit site — inside `reservation_lifecycle_state` and nowhere else.

The residual branch is **not** a single collapsed member. Invalid input can carry `NoPayloadFactsValue` — the envelope explicitly has no payload facts — or `DeliberatelyUninspectedPayloadValue`, where the coordinator chose not to inspect a payload domain. Collapsing them erases a distinction the preceding phase audited and shipped. The typed identity, foreign-resolution, and unknown-reservation branches all ignore `message`; **the residual branch keeps its message-backed diagnostic**, precisely because no semantic payload facts exist there and the diagnostic string is the only thing that does.

**The `ledger_unreadable` reading carries the typed replay failure, and its generic branch means no facts.** `_replay_failure_classification` (`:3515`) returns a `ReplayFailure` (`:939`) reached through `.tagged()` and is called from all four classification entry points — `classify_claim` (`:2258`), `classify_check` (`:2462`), `_validate_board` (`:2520`), and `coordinator_state` (`:2860`) — ahead of each one's generic `ledger_unreadable` branch. It keys on `envelope.status == "ledger_unreadable"` with `payload.kind == "replay_failure"`, so it belongs to the `ledger_unreadable` reading rather than the invalid-input union, and its `ReplayFailureValue` (`:930`) already exists — lift it rather than rebuilding it. The generic branch beneath it carries **no payload facts**: `parse_envelope` (`:1495`) applies `valid_outcome_tuple` (`:1556`) before anything else, so a malformed replay payload never crosses validation and can never reach that branch. Type it as the no-facts reading, and keep it below the typed classifier for the same ordering reason the invalid-input branch must.

**Success-tagged lookalikes are rejected at the boundary and must stay that way.** `parse_envelope` rejects a success-tagged envelope carrying a rejection-only nested status before any classifier runs, and the existing identity fixture and the foreign-resolution test (`tests/test_hook_rendering.py:2552` and `:2591`) already expect `EnvelopeValidationError`. Do **not** require these to reach a caller as ordinary engine responses, and do **not** construct an impossible `ValidatedEnvelope` merely to exercise classifier ordering — an exhaustive match over a union whose invalid members cannot be built is the property to preserve, not a fixture that manufactures one.

**Phase 7 left the generic `invalid_input` branch order-sensitive and that ordering is a correctness constraint, not a style preference.** `sequence` and `integrate` identity rejections arrive with status `invalid_input`, so the generic `invalid_input` early return (its generic branch at `:2882`) must stay *below* the two typed classifiers (`:3503` and `:3528`); placed above them, the typed identity states are silently erased into a diagnostic string. A tagged union whose members are matched exhaustively removes the hazard rather than preserving the ordering — prefer that, and do not ship a version that merely re-encodes the current sequence of `if` returns.

**The hook states the session-id contract the engine now enforces.** Phase 18 made the PostToolUse payload parser reject a `session_id` that is overlong or carries control characters, where it previously rejected only an empty one, and `berth_post_bash.sh:422` still tells the user only that `session_id` must be non-empty. Correct that message to state the real contract — 1 to 256 characters, no control characters — and add exact rendering tests for both rejections. The hook does not revalidate an already-typed id; it renders the engine's rejection. `run_installed_engine_hook` (`tests/test_hook_rendering.py:917`) exists but hardcodes its payload through `post_bash_payload_for`, so it cannot express these cases — extend it to take an explicit session id, and drive the 256-character multibyte, 257-character, and control-character cases through it.

Project rules that bind here: never use file-level type ignores; avoid `Any`; line-level `# pyright: ignore[reportAny]` is a last resort on the specific line only.

**Files:**
- `/Users/natemccoy/.claude/scripts/berth/claim_state.py` — the five classifier returns, the four `tagged()` conversions, the rendered recovery-action and identity-rejection unions, the two contention types, the typed `render_board` input, and the exhaustive envelope readings
- `/Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py` — this phase's classifier, board-parity, and hook fixtures, and the extended `run_installed_engine_hook`
- `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh` — the session-id rejection message at `:422`
- `/Users/natemccoy/.claude/scripts/berth/generated/status_payload_tables.py` — read-only input: the generated closed sets
- `crates/cargo-berth/src/board/mod.rs` — read-only input: the engine side of the board contract this phase must not change
- `crates/cargo-berth/src/session/mod.rs` — read-only input: the session-id rule the hook message must state
- `docs/cargo-berth/json-contract.md` — read-only input: the published `cargo-berth-claim-state/v1` shape the parity fixture holds to

**Constraints from prior phases:** The preceding phase gave every envelope alert a named owner and proved the four recovery commands reach it; consume that ownership rather than re-deriving alerts here. The phase before it typed the envelope boundary: the wire value echoes the payload exactly, a separate semantic reading carries what was read, `EnvelopePayload` no longer exists — `ValidatedEnvelope` owns `value` and `reading` — the three envelope discriminants and all three `expected_verb` sites already take generated aliases, and every operation's arguments already arrive as a `CoordinatorRequest` member. `BlockedEditPayloadReadingValue` remains the union alias over its check and claim members at `:1000`. Do not rebuild any of that; consume it. Phase 20's generated module is the only source for a closed set — a hand-copied literal set is the defect Phase 14 exists to prevent. **`_inactive_identity_classification` no longer exists** — Phase 12 replaced it with `_coordination_identity_classification` and `_foreign_actor_incursion_resolution_classification`; do not key any edit or call-site sweep on the old name. Phase 12 also bound both classifiers to the envelope's own status, refusing any envelope whose status is not `invalid_input`, which is the property this phase's exhaustive match must preserve rather than rediscover. Phase 14 added `OutputStatus::LegacyHookOutdated` as its own wire status, so `ledger_unreadable` is no longer the catch-all it was when an earlier version of this Work Order was written. Phase 17's four harness guards in `test_hook_rendering.py` are preserved rather than rediscovered, and `HookTimingTests` already exists as its own separately invocable class — do not split it again. The files in this phase cannot join a `cargo-berth` checkpoint commit; establish a pre-edit snapshot of them and diff against it rather than against `HEAD`, and run tooling from `/Users/natemccoy/.claude` so its `pyrightconfig.json` applies.

**Acceptance gate:** `basedpyright scripts/berth/claim_state.py scripts/berth/tests/test_hook_rendering.py scripts/berth/generated/status_payload_tables.py`, run with `/Users/natemccoy/.claude` as the working directory, reports **zero errors, zero warnings, and zero notes** for all three files. `python3 /Users/natemccoy/.claude/scripts/berth/tests/test_hook_rendering.py HookRenderingTests` passes in full; the timing class is invoked separately and its result is reported, not gated.

All five classifiers and all four `tagged()` methods carry exact named return annotations, no `dict[str, object]` among them. No `cast` stands between a tagged value and its return, and none remains in state dispatch. No file-level type ignore exists in any gated file, and every remaining `# pyright: ignore` is line-level with a named rule.

A fixture carrying Phase 13's `unknown_reservation` rejection reaches a caller as `UnknownReservationLifecycleQueryRejectionValue`, not as a residual member, and the conversion to the normalized form is proved to occur only inside `reservation_lifecycle_state`. The residual invalid-input branch is proved to keep both `NoPayloadFactsValue` and `DeliberatelyUninspectedPayloadValue` as distinct readings, with its message-backed diagnostic intact, while the typed identity, foreign-resolution, and unknown-reservation branches are proved to read no `message`.

Replay fixtures exercise `classify_claim`, `classify_check`, `_validate_board`, `render_board`, and `coordinator_state`; each returns a typed `ReplayHardStopStateValue` carrying typed reason, subject, effect, and operator route, with no `dict` return, no `cast`, and no `message` read, while a generic `ledger_unreadable` carrying no payload facts stays distinct from it. Fixtures reach each of `EngineInvocation`, `CoordinationIdentityRejection`, and `ForeignActorIncursionResolution` as a typed member rather than through a `tagged()` mapping.

The contention state and the retry-ready contention state are distinct types, and a test fails if `command_to_rerun` is ever assigned after classification returns.

`render_board` consumes the typed board value and reads no raw mapping, proved by a test that fails if the wire envelope is reachable from its body. A parity fixture proves the emitted `cargo-berth-claim-state/v1` JSON is equivalent to the pre-change output — same markdown order, same paths, same action values.

The rendered recovery-action union has exactly one member per action, proved by an exhaustive check that fails when a fifth action is added and when any tag is widened back to `str`; the rendered identity-rejection union likewise has one member per rejection, paired with an exhaustiveness check over the generated `COORDINATION_IDENTITY_REJECTION_KINDS`, so a rejection kind added to the engine fails this phase's tests rather than reaching a user unrendered. Fixtures cover **every** lifecycle alternative and **every** release disposition — one each for `active`, `outstanding`, `released_after_checkpoint`, and `released_without_checkpoint`, and one each for `integrated`, `rewritten_integration`, `abandoned`, and `retired_orphan` — each reaching a caller as its own typed member.

The existing success-tagged lookalike fixtures continue to raise `EnvelopeValidationError` at the parse boundary, and no test constructs a `ValidatedEnvelope` that `parse_envelope` would reject.

The hook renders the corrected session-id contract, proved against the **installed engine** through the extended `run_installed_engine_hook` rather than the `run_hook` fixture — that fixture returns its configured exit independently of the payload it is handed, so a rejection-message test written against it passes without ever reaching the engine parser. A 256-character multibyte id produces no invalid-payload feedback; a 257-character id and an id carrying a control character each produce the exact corrected message at `berth_post_bash.sh:422`, naming the 1-to-256-character no-control-character rule. Neither path revalidates an already-typed id. The four Phase 17 harness guards are intact. The phase's own diff is read against the pre-edit snapshot of the named files, not against `HEAD`.
