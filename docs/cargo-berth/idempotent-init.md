# idempotent-init — `cargo berth init` enrolls the worktrees already in flight

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** `init` writes the configuration where every worktree reads it and enrolls every live worktree that has never held a reservation with one reservation covering its footprint since its merge-base with trunk; overlaps between footprints hold integration until answered with `sequence`; re-running `init` is safe and enrolls worktrees added since. Scope (user-set): one user, their own repositories, a handful of linked worktrees, Claude Code sessions as callers — unlikely edge cases get no mechanism.

## Delegation Context

- **Project:** cargo-berth — a reservation engine for git worktrees. `init` sets up the ledger, config, and managed hooks; `claim`, `check`, `drift`, `sequence`, and `integrate` coordinate paths across worktrees. Binary crate: integration tests drive `CARGO_BIN_EXE_cargo-berth`; private APIs are tested with unit tests in the source file.
- **Project started:** 2026-09-15T15:46:38.647+00:00
- **Stack:** Rust edition 2024 (workspace, resolver 3). clap 4.6.6 (derive), serde 1 (derive), serde_json 1, schemars 1, uuid 1 (v7). Dev: cargo-berth-test-support (`GitDriver`, `git_command`), tempfile 3.27.0. Lints: clippy all/pedantic/nursery/cargo deny; `unwrap_used`/`expect_used`/`panic` deny; `missing_docs` deny; `self_named_module_files` deny (a module with children uses `mod.rs`; a leaf module is `name.rs`).
- **Layout:** `crates/cargo-berth/src/`: `cli.rs`, `config.rs`, `output.rs`, `output_contract.rs`, `ledger/` (journal, handle, worktree_context, authorization), `reservation/` (retention, partition), `answer/` (conflict_authorization, scope_binding), `edge/graph.rs`, `verb/` (claim, sequence), `drift/observation.rs`, `session/mod.rs`, `worktree/` (mod, liveness), `git/reachability.rs`, `board/` (rows, answers). Tests: `crates/cargo-berth/tests/`. Docs: `crates/cargo-berth/README.md`, `docs/cargo-berth/operations.md`, `docs/cargo-berth/generated/output-contract.json`.
- **Key files:**
  - `crates/cargo-berth/src/ledger/journal.rs` — `enum JournalOperation` :337 (`Claim.phase_start_head: ProtectedPhaseStartHead` :353); `enum ClaimSource` :570 (`WorkPlan`, `FirstTouch`, `Explicit`, `Enrolled`); `append_events` :1741.
  - `crates/cargo-berth/src/ledger/handle.rs` — `TransactionValidation` :102; `Ledger::initialize` :177 (discovers the invoking `WorktreeContext` and calls `BerthConfig::initialize` :182 with its `configuration_lookup()`); `Ledger::transact` :260 (one operation, actor worktree + run, validated against locked replay); `RecordTooLarge` rejection for records over `MAXIMUM_JOURNAL_RECORD_BYTES` (16 KiB, `src/ledger/constants.rs:19`).
  - `crates/cargo-berth/src/ledger/worktree_context.rs` — `configuration_lookup` :255 (`ConfigurationLookup::OwnThenMain`: a linked worktree reads its own file, then the main worktree's); `publish_coordination_run_marker` :271.
  - `crates/cargo-berth/src/ledger/authorization.rs` — `EditAuthorization::resolve_from_sources` :158 (an unmapped session resolves the worktree's run marker).
  - `crates/cargo-berth/src/config.rs` — `pub(crate) enum Enrollment<T>` :26 (repository configuration presence — unrelated to worktree enrollment, not renamed); `ConfigurationFilePresence` :74 (`Missing`, `Present(BerthConfig)`); `BerthConfig::initialize(&ConfigurationLookup)` :120 (writes `.claude/config/berth.toml` at the main worktree root, carrying a validated linked file's policy when the main file is absent and keeping a present main file); `BerthConfig::read` :172 (already falls back from a linked worktree's own file to the main worktree's, own file first; unit test `a_linked_worktree_without_its_own_file_reads_the_main_worktree` :645); `read_file` :197 (returns `Result<ConfigurationFilePresence, ConfigError>`).
  - `crates/cargo-berth/src/cli.rs` — `Init` dispatch → `initialize_ledger` :1425; `SequenceArguments` :562.
  - `crates/cargo-berth/src/answer/conflict_authorization.rs` — `enum ConflictAuthorization` :21 (`NoConflict`, `Enrollment { overlaps }`, `Sequence`, `Defer`, `Override`, `ExistingAnswersCoverEveryOverlap`); `covers` :105 (`Enrollment` matches counterpart plus recorded shared scopes and ignores `scope_revision`; every other variant keeps revision equality).
  - `crates/cargo-berth/src/answer/scope_binding.rs` — `AuthorizedOverlap` :30 (`reservation_id`, `scope_revision`, `scopes`); `AuthorizedOverlapSet` :42 (non-empty); `impl From<&ReservationConflict> for AuthorizedOverlap` :73 (copies holder id, protection revision, shared scopes); `covers` :84; `covers_shared_scope` :96.
  - `crates/cargo-berth/src/answer/proposal.rs` — `OverlapAuthorizationReason::enrollment()` :159 (engine explanation carried by enrollment deferrals).
  - `crates/cargo-berth/src/reservation/partition.rs` — `reservations_authorize_scope` :155 (checks both reservations' authorizations; used by edit checks and widening).
  - `crates/cargo-berth/src/reservation/retention.rs` — `acting_head_containment: Option<ActingHeadContainment>` :153, set by `with_acting_head_containment` :279; `conflicts_for_claim` :330 (shared scopes and holder protection per foreign reservation); `apply_claim` :1098 (Claim replay sets `MergeExtent::NotDerived` :1123); `protection_for_conflict` :1488.
  - `crates/cargo-berth/src/reservation/merge_extent.rs` — `observed_unmerged_work` :128; `protected_key` :140 (returns `Option<&MergeExtentKey>`).
  - `crates/cargo-berth/src/reservation/containment.rs` — `ActingHeadContainment::observe` :41 (read before any lock: a foreign holder protects only what its head would still bring to the acting HEAD, plus its uncommitted paths).
  - `crates/cargo-berth/src/reconcile.rs` — `append_merged_run_endings` :3085 (ends an active run with no release command once it has done work — `observed_unmerged_work()` or HEAD past `phase_start_head` — and the pass observes `MergeExtent::Empty` at a head trunk contains).
  - `crates/cargo-berth/src/edge/graph.rs` — `struct DeferredOverlap` :41; `prepare_deferred_edge` :260; `apply_authorization` :352 (projects `Defer` and `Enrollment` deferrals, stamping `DeferralOrigin`); `apply_resolution` :445; `deferred_overlap_between` :502 (either orientation); `AmbiguousDeferral` :527.
  - `crates/cargo-berth/src/edge/mod.rs` — `enum DeferralOrigin` :192 (`UserAnswer`, `Enrollment`; carried on `IntegrationDeferralConstraint.origin`).
  - `crates/cargo-berth/src/verb/sequence.rs` — `execute_sequence` :130 (resolves a pending deferral in either order into an edge).
  - `crates/cargo-berth/src/verb/claim.rs` — `PhaseStartSelection::Protected` :137; `ClaimRepositoryFacts` :140 (private, needs `ClaimRunValidation` — do not use for enrollment); `select_first_touch_reservation_reuse` :1045 (accepts any eligible active reservation, whatever its source); `PreparedClaim::into_operation` :1405 (model for building a `Claim` operation).
  - `crates/cargo-berth/src/drift/observation.rs` — `observe_merge_working_tree` :522 (exported; `tracked_paths`, `untracked_paths`; the one status read shared by merge-extent dirty evidence and enrollment footprints).
  - `crates/cargo-berth/src/git/reachability.rs` — `unmerged_branch_paths` :349.
  - `crates/cargo-berth/src/worktree/liveness.rs` — `WorktreeRegistry` :90 (private `WorktreeRegistration` with `WorktreeRegistrationState::{Available, Locked, Prunable}` and `RegisteredWorktreeLocation::{Discovered, Unavailable}`), impl :139; `marker_sweep_contexts` :210 (marker-specific; omits unavailable registrations).
  - `crates/cargo-berth/src/session/mod.rs` — `apply_journal_event` :286 (publishes every claim's session mapping into the invoking session).
  - `crates/cargo-berth/src/output.rs` — `source_description` :4197 (exhaustive `ClaimSource` match); `OutputFacts::Init(InitializationPayload)` :717; `InitializationPayload` :756.
  - `crates/cargo-berth/src/board/answers.rs` — `RecordedAnswer` :33 (`Defer` :49, `Enrollment`); `AnswerAcquisition` :93 (`Enrollment`). `crates/cargo-berth/src/board/rows.rs` — `UnresolvedOverlap` :273 (`origin`: `user_answer` / `enrollment`); `unresolved_overlaps` :627.
  - `crates/cargo-berth/src/output_contract.rs` — test `generated_artifacts_are_reproducible_from_the_checked_in_contract` :312. `ClaimSource` and authorization kinds appear in the generated schema; regenerate with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1 cargo nextest run -p cargo-berth generated_artifacts_are_reproducible_from_the_checked_in_contract`.
  - Existing behavior the plan relies on: `tests/overlap.rs:122` and `tests/lifecycle.rs:1176` (a session joins a worktree's existing run through its marker and reuses its reservation with no append); `tests/hooks.rs:815` (a worktree added after `init` reads the main config). Worktree helpers: `tests/hooks.rs` `add_worktree` :1673 and `add_worktree_without_configuration` :1689, `tests/edges.rs` `add_worktree` :2479, `tests/ledger.rs` `add_worktree` :1022.
  - Test environment: the shared cargo target directory can serve integration binaries built from another worktree; when an integration result contradicts the code, `touch crates/cargo-berth/tests/*.rs` and rerun the listed `verify.sh test` line.
- **Test lanes:** cargo-berth — `crates/cargo-berth/tests`
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  - The journal is truth; enrollment state is a replay query (plan).
  - Decide and record in one mutation-lock hold; no git inside the lock (repository).
  - Existing overlap answers keep their behavior; the new enrollment variant never revokes edit access (plan).
  - Journal history is never removed. An enrolled reservation ends like any other run: through checkpoint and release, or automatically once its checkout is clean with no net branch change at a head trunk contains (`src/reconcile.rs:3085`) (user-set: runs end automatically once their work reaches trunk).
  - Older journals replay unchanged; new variants are additive; bytes under `tests/fixtures/reader_compat` stay unchanged (repository).
  - Each journal record is at most 16 KiB (repository).
  - The output wire contract grows by addition only; `output-contract.json` matches the generator (repository).
  - Out of scope (user-set single-user scope): automatic enrollment at later hook contacts (re-run `init` instead), unordered overlap dismissal, capacity limits during enrollment, footprints too large for one record (reported, not split), config disagreement reporting, crash recovery mid-`init`, gate subject semantics, squash merges, stashes, bare repositories, cross-clone coordination.

## Phases

<!-- PHASE NUMBERING — binds every command that edits this doc.
     N is a bare integer, numbered from 1, contiguous, in execution order.
     Never a letter suffix (`4a`, `2b1a`) — those stop sorting and make ranges
     unreadable. Any spine edit (insert, split, merge, reorder, delete)
     resequences from the edit point to the end AND updates every cross-reference:
     substitute highest-first so `Phase 8` does not corrupt `Phase 12`, and
     re-check that each `Phases X–Y` range still spans the same set.
     Insert after the last `done` phase where possible — a done phase's number
     is baked into its checkpoint commit message and cannot be rewritten; if one
     must be renumbered, add an old→new mapping note to the doc.
     Full procedure: /plan:to_phased_plan → <PhaseNumbering/>. -->

### Phase 1 — Enrolled claims and enrollment overlap authorization  · status: done

#### As-built

- The journal records an enrolled `Claim` (`ClaimSource::Enrolled`, wire `enrolled`) carrying `ConflictAuthorization::Enrollment { overlaps: AuthorizedOverlapSet }`, which binds each counterpart reservation and the shared scopes recorded at enrollment. It has no blocker, reason field, or cause enum.
- `ConflictAuthorization::covers` routes Enrollment to `AuthorizedOverlap::covers_shared_scope`: the counterpart matches and the scope is among the recorded shared scopes, with `scope_revision` ignored. Every other variant keeps revision equality through `AuthorizedOverlap::covers`. An unrelated widen on either side does not re-block the pair; a new shared path gets ordinary treatment. Editing goes through the existing `reservations_authorize_scope` path.
- `OrderingGraph::apply_authorization` projects one `DeferredOverlap` per counterpart via a shared `add_deferral` helper, stamped with `edge::DeferralOrigin::Enrollment` and the engine reason `OverlapAuthorizationReason::enrollment()`. Existing `IntegrationHold::DeferredOverlap` holds and `sequence <first> <then> --why` resolution apply unchanged in either order.
- The board reads `DeferralOrigin` rather than rescanning the journal. It lists enrollment answers per unresolved pair until each pair is sequenced; a resolved pair appears as `OrderingCreatedFromDeferral` carrying the engine reason. The output contract grows by addition only.

**Files:**
- `crates/cargo-berth/src/ledger/journal.rs` — `ClaimSource::Enrolled`.
- `crates/cargo-berth/src/answer/conflict_authorization.rs` — `Enrollment` variant and its coverage rule; unit tests.
- `crates/cargo-berth/src/answer/scope_binding.rs` — `AuthorizedOverlap::covers_shared_scope`.
- `crates/cargo-berth/src/answer/proposal.rs` — `OverlapAuthorizationReason::enrollment()`.
- `crates/cargo-berth/src/edge/mod.rs` — `DeferralOrigin`, `IntegrationDeferralConstraint.origin`.
- `crates/cargo-berth/src/edge/graph.rs` — Enrollment projection, origin stamping, `add_deferral`; unit tests.
- `crates/cargo-berth/src/board/answers.rs`, `crates/cargo-berth/src/board/rows.rs` — enrollment labels.
- `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/verb/claim.rs` — `source_description` and exhaustive matches (`FirstTouch | Enrolled => into_exact_file_antichain`).
- `docs/cargo-berth/generated/output-contract.json` — `enrolled` source, Enrollment authorization, `DeferralOrigin` `oneOf`, new board branches.

**Binds later work:** `ClaimSource::Enrolled` (wire `enrolled`). `ConflictAuthorization::Enrollment { overlaps: AuthorizedOverlapSet }` covers counterpart + recorded shared scopes via `AuthorizedOverlap::covers_shared_scope`, ignoring `scope_revision`; `AuthorizedOverlap { reservation_id, scope_revision, scopes }` still requires `scope_revision` on the wire. `edge::DeferralOrigin { UserAnswer, Enrollment }` on `DeferredOverlap` and `IntegrationDeferralConstraint.origin`. `OverlapAuthorizationReason::enrollment()` wording is not a contract. Board JSON: `UnresolvedOverlap.origin` (`"user_answer"` / `"enrollment"`), `RecordedAnswer::Enrollment { reservation_id, exact_approved_scopes, acquisition, consequence }`, `AnswerAcquisition::Enrollment`. Enrolled reservations end automatically once their work reaches trunk, like any run.

**Gotchas:** The shared cargo target dir can serve stale integration binaries built in another worktree; `touch crates/cargo-berth/tests/*.rs` forces a rebuild. `reservations_authorize_scope` (`src/reservation/partition.rs`) consults holder-side authorization only when the requester's foreign protection is Protected, so a clean counterpart with an `Empty` merge extent cannot edit the shared path through the other side's Enrollment authorization (identical for `Defer`). A new enum variant's exhaustive match sites, `verb/claim.rs` included, belong to the variant owner.

**Ruled out:** Preserving editing for a clean active counterpart through the other side's Enrollment authorization (rare; a contained counterpart ends automatically). A worktree-specific `CARGO_TARGET_DIR` in acceptance gates (`verify.sh` owns commands; the stale-binary note covers it).

### Phase 2 — Configuration at the main worktree root  · status: done

#### As-built

- `BerthConfig::initialize(&ConfigurationLookup)` writes `.claude/config/berth.toml` at the main worktree root for `ConfigurationLookup::OwnThenMain`, and at the invoking root for `ConfigurationLookup::Own`.
- A present, valid main-root file is kept (`InitializationState::Existing`). When it is absent and the invoking linked worktree has its own validated `berth.toml`, that file's `trunk`, `gate_mode`, `maximum_reservations`, and `maximum_ordering_edges` are serialized into the new main file; otherwise defaults are written. The linked file stays and keeps precedence for its worktree.
- Private `ConfigurationFilePresence::{Missing, Present(BerthConfig)}` is `read_file`'s result at the filesystem boundary; `read` falls back to the main root by re-reading it with `ConfigurationLookup::Own`.
- `Ledger::initialize` discovers `WorktreeContext` before the initialization lock and passes its `configuration_lookup()`. The CLI, `InitializationPayload`, and the output contract are unchanged.

**Files:**
- `crates/cargo-berth/src/config.rs` — initialize target and linked-policy carry-over, `ConfigurationFilePresence`, unit tests including `invalid_linked_policy_leaves_the_main_configuration_missing`.
- `crates/cargo-berth/src/ledger/handle.rs` — `Ledger::initialize` passes the lookup.
- `crates/cargo-berth/README.md` — First use line: `init` creates the configuration at the main worktree root, and linked worktrees without their own read it.
- `crates/cargo-berth/tests/hooks.rs` — `init_from_a_linked_worktree_writes_the_main_root_configuration`, `init_carries_a_linked_configuration_policy_to_the_main_root`, `init_keeps_a_present_main_root_configuration`.

**Binds later work:** `init` writes the main-root `berth.toml` via `configuration_lookup` (`ConfigurationLookup::OwnThenMain`); linked-policy carry-over covers `trunk`, `gate_mode`, `maximum_reservations`, `maximum_ordering_edges`; a present main file is kept; linked worktrees without their own file read the main file; `read_file` returns `ConfigurationFilePresence::{Missing, Present(BerthConfig)}`.

**Gotchas:**
- An invalid linked `berth.toml` fails `init` from that worktree before the main file exists; a present main file is accepted without reading the linked file.
- `tests/hooks.rs` `add_worktree` copies the main configuration into the new worktree; linked-only or unconfigured fixtures use `add_worktree_without_configuration` or remove the main file after adding.

**Ruled out:** passing a bare main-root path to `initialize` (loses the linked carry-over source).

### Phase 3 — Worktree enrollment in `init`  · status: todo

#### Work Order

**Goal:** `cargo berth init` enrolls every never-reserved live worktree with one reservation per worktree; a second run appends nothing new.

**Spec:**
- **Module.** New leaf module `src/worktree/enrollment.rs` (`mod enrollment;` in `src/worktree/mod.rs`). Its aggregate result type is `WorktreeEnrollmentReport`; candidate skips and failures are semantic variants, not optional result fields.
- **History query.** A worktree has reservation history when replay holds any `Claim` whose actor worktree is its id, whatever source or lifecycle. Such a worktree is never enrolled.
- **Candidates.** `WorktreeRegistry` (`src/worktree/liveness.rs:90`) exposes a typed enrollment enumeration from its existing single registry read: eligible candidates (registration `Available` or `Locked`, location `Discovered`, with their `WorktreeContext`) and unavailable roots (`Prunable` or `Unavailable`), which are reported as failures. Do not reuse `marker_sweep_contexts` (:210).
- **Footprint**, per candidate, outside the lock:
  1. Resolve trunk and `HEAD` object ids once.
  2. If `rebase-merge`, `rebase-apply`, `MERGE_HEAD`, `CHERRY_PICK_HEAD`, or `REVERT_HEAD` exists in that worktree's git dir, report `operation_in_progress`.
  3. Merge-base of trunk and `HEAD`; none, trunk unresolved, or unborn `HEAD` reports `no_merge_base`.
  4. Committed: `git::unmerged_branch_paths(trunk, head)` (`src/git/reachability.rs:349`).
  5. Working tree: `drift::observe_merge_working_tree` (`src/drift/observation.rs:522`) `tracked_paths` ∪ `untracked_paths`, in that worktree's root.
  6. Union as exact file scopes. Empty: not enrolled, not reported.

  A failing git command reports `git_failure` with the command and error; other candidates proceed.
- **Configuration is not work.** `observe_merge_working_tree` omits an untracked `.claude/config/berth.toml` under the observed root, so neither enrollment footprints nor merge-extent dirty evidence count the file `init` creates; tracked changes to that path still count. A clean repository without an ignore rule for the file enrolls nothing, and a main-worktree run can still end automatically.
- **Stacked containment.** Overlaps use the edit check's containment rule (`ActingHeadContainment`, `src/reservation/containment.rs:41`): a shared path forms an enrollment overlap only when each side would still bring it to the other — it is in that side's working-tree paths, or in `git::unmerged_branch_paths(other_head, this_head)`. Compute these pairwise remainders outside the lock for candidate pairs, and `ActingHeadContainment::observe` for existing holders, with the footprints. A worktree stacked on another in-flight branch therefore forms no pair for the parent's commits it already contains; dirty or independently changed shared work keeps its pair. Reservation footprints themselves stay full (trunk-relative).
- **Enrollment transaction**, one `Ledger::transact` (`src/ledger/handle.rs:260`) per candidate, actor = candidate worktree id and a newly issued run. Inside validation: skip if the locked replay shows history; replay reservations (earlier candidates' claims included), attach the precomputed containment, call `RetainedReservationSet::conflicts_for_claim(&footprint, candidate_worktree_id, path_case)` (`src/reservation/retention.rs:330`), drop shared scopes the pairwise remainders show as contained, and convert each remaining conflict with `AuthorizedOverlap::from` (`src/answer/scope_binding.rs:73`) — `scope_revision` stays on the wire although `Enrollment` coverage ignores it. Append one `Claim` with `source: ClaimSource::Enrolled`, `purpose` naming enrollment and the branch or detached head, `phase_start_head` = merge-base as `ProtectedPhaseStartHead` (`journal.rs:353`), `head_snapshot` and `trunk_at_claim` from step 1, `worktree_root` and `worktree_administrative_locator` from the candidate's `WorktreeContext`, `coordination_identity_provenance: NotPresented`, `authorization` = `NoConflict` for no conflicts or `Enrollment { overlaps: AuthorizedOverlapSet }`. Build the operation directly, modeled on `PreparedClaim::into_operation` (`src/verb/claim.rs:1405`); do not use `ClaimRepositoryFacts`, and do not copy `validate_claim_transaction`'s Git observation into validation — all Git facts are read before the lock. A `RecordTooLarge` rejection reports `record_too_large` for that worktree. Session mapping publication (`session::apply_journal_event`, `src/session/mod.rs:286`) is skipped for enrolled claims so the invoking session is not mapped to sibling reservations.
- **Containment type.** Replace `RetainedReservationSet.acting_head_containment: Option<ActingHeadContainment>` (`retention.rs:153`) with a named policy — `FullProtection` or `ExcludeContainedWork(ActingHeadContainment)` — and match existing `MergeExtent` states rather than adding new callers of `protected_key()`'s `Option` (`merge_extent.rs:140`).
- **Automatic ending.** An enrolled reservation ends like any other run (`append_merged_run_endings`, `src/reconcile.rs:3085`). Enrollment itself counts as having done work — its footprint was non-empty — and replay keeps that as a named state that later empty drift or gate observations do not erase. So an enrollment on trunk whose edits are later committed to trunk or discarded ends automatically once its checkout is clean at a head trunk contains. An unresolved enrollment deferral stays pending after either endpoint ends, until `sequence` resolves it (`graph.rs`, `board/rows.rs:627`).
- **Run marker.** After each claim succeeds, publish its run with `publish_coordination_run_marker` (`worktree_context.rs:271`) in that worktree. Sessions there then join the run through existing identity resolution and reuse the reservation with no append (`tests/overlap.rs:122`); first-touch reuse (`select_first_touch_reservation_reuse`, `verb/claim.rs:1045`) and drift attribution are already source-independent.
- **Output.** Additive enrollment section on `InitializationPayload` (`src/output.rs:756`); regenerate `output-contract.json`:
  - enrolled: reservation id, worktree root, branch or detached head;
  - overlaps: both reservation ids, shared scopes, and the two ready-to-run `sequence <a> <b> --why <text>` commands (both orders);
  - failures: worktree root, reason (`operation_in_progress`, `no_merge_base`, `git_failure`, `unavailable`, `record_too_large`), and diagnostic.
  - Worktrees with history or empty footprints are omitted. The single legacy `init` line is emitted only when enrolled, unresolved enrollment overlaps, and failures are all empty.
- **Idempotence.** A re-run enrolls only worktrees still without history (added since, or previously failed) and re-reports unresolved enrollment overlaps, including a pair whose endpoint has since ended.
- **Docs.** `crates/cargo-berth/README.md` "First use": `init` on a repository with work in flight, reading the overlap report, answering with `sequence`, and re-running `init` after adding a worktree that already has commits, before its first edit. `docs/cargo-berth/operations.md`: a short enrollment section (config location, failure reasons and what to do).

**Files:**
- `crates/cargo-berth/src/worktree/mod.rs` — `mod enrollment;`.
- `crates/cargo-berth/src/worktree/enrollment.rs` — new: `WorktreeEnrollmentReport`, history query, candidates, footprint, pairwise containment, per-candidate transaction, run marker.
- `crates/cargo-berth/src/worktree/liveness.rs` — typed enrollment enumeration.
- `crates/cargo-berth/src/session/mod.rs` — skip mapping publication for `ClaimSource::Enrolled`.
- `crates/cargo-berth/src/drift/observation.rs` — untracked configuration file is not work.
- `crates/cargo-berth/src/reservation/retention.rs` — containment policy type; enrolled work evidence in replay.
- `crates/cargo-berth/src/reservation/containment.rs` — containment policy type, if it lives here.
- `crates/cargo-berth/src/reservation/merge_extent.rs` — enrolled work evidence, if it lives in the extent.
- `crates/cargo-berth/src/reconcile.rs` — automatic ending counts enrollment as work.
- `crates/cargo-berth/src/cli.rs` — `initialize_ledger` :1425 runs enrollment after config and hooks.
- `crates/cargo-berth/src/output.rs` — enrollment section.
- `docs/cargo-berth/generated/output-contract.json` — regenerated.
- `crates/cargo-berth/README.md` — First use.
- `docs/cargo-berth/operations.md` — enrollment section.
- `crates/cargo-berth/tests/ledger.rs` — integration tests.
- `crates/cargo-berth/tests/gate.rs` — integration tests.

**Seats:** 2 writers + 1 tester — enrollment engine split from CLI/output/docs.
- `impl` — `src/worktree/{mod.rs,enrollment.rs,liveness.rs}`, `src/session/mod.rs`, `src/drift/observation.rs`, `src/reservation/{containment.rs,retention.rs,merge_extent.rs}`, `src/reconcile.rs`; hub: `src/worktree/enrollment.rs` (define `WorktreeEnrollmentReport` first — output renders it).
- `test` — `tests/ledger.rs` and `tests/gate.rs`:
  - `tests/ledger.rs`, one three-worktree scenario (two with committed and dirty edits to one shared file, one on trunk with dirty edits) run with `init` from a linked worktree: three reservations with journal `source.kind = enrolled`; the init report lists them; the overlap is reported with two `sequence` commands; `cargo-berth board --json` shows the unresolved overlap with `origin: "enrollment"` and a recorded `enrollment` answer with acquisition origin `enrollment`; one rendered `sequence` command runs as printed, resolves the pair, and the board shows the ordering created from that deferral; second `init` appends nothing and re-reports any still-unresolved pair; a worktree whose reservation was released is not re-enrolled; a worktree mid-rebase reports `operation_in_progress`; a worktree added after `init` with commits enrolls on the next `init`; an unavailable registered worktree is reported while a locked one enrolls.
  - `tests/ledger.rs`, stacked: a worktree branched from another in-flight branch, touching only its own paths, enrolls with no overlap pair against the parent.
  - `tests/ledger.rs`, automatic ending: the trunk worktree's enrollment ends automatically after its dirty edits are discarded; an unresolved pair with an ended endpoint is still re-reported by `init`.
  - `tests/ledger.rs`, configuration: `init` in a clean repository with no ignore rule for `.claude/config/berth.toml` enrolls nothing and prints the legacy line.
  - `tests/ledger.rs`, sessions: with `CARGO_BERTH_SESSION_ID` set, `init` leaves the invoking session's mapping unchanged; an unmapped session in an enrolled worktree edits a covered path with no new `Claim` or `Widen`, and widens that same reservation for a new unclaimed path.
  - `tests/gate.rs`: the unresolved enrollment pair holds integration (reported under observe, rejected under enforce), and both sides edit the shared file with no incursion.
- `review` — opens as impl: `src/cli.rs`, `src/output.rs`, `docs/cargo-berth/generated/output-contract.json`, `crates/cargo-berth/README.md`, `docs/cargo-berth/operations.md`; regenerates the contract after both writers post `done`.

**Constraints from prior phases:**
- Phase 1 added `ClaimSource::Enrolled` (wire `enrolled`; `source_description` and the `verb/claim.rs` exhaustive match already handle it) and `ConflictAuthorization::Enrollment { overlaps: AuthorizedOverlapSet }`. `covers` (`conflict_authorization.rs:105`) matches counterpart plus recorded shared scopes via `AuthorizedOverlap::covers_shared_scope` and ignores `scope_revision`, but `AuthorizedOverlap` still requires `scope_revision` on the wire.
- Replay projects one `DeferredOverlap` per counterpart stamped `DeferralOrigin::Enrollment` (`edge/mod.rs:192`) with the engine reason `OverlapAuthorizationReason::enrollment()` (`answer/proposal.rs:159`); existing holds and `sequence` resolution apply. Board JSON already labels it: `UnresolvedOverlap.origin` (`user_answer` / `enrollment`), `RecordedAnswer::Enrollment { reservation_id, exact_approved_scopes, acquisition, consequence }`, `AnswerAcquisition::Enrollment`; enrollment answers stay listed per unresolved pair until each pair is sequenced. The reason's wording is covered by unit tests, not a text contract.
- Holder-side authorization in `reservations_authorize_scope` (`partition.rs:155`) is consulted only when the requester's foreign protection is `Protected`; unchanged here (author scope note: a clean active counterpart is rare, and a contained one ends automatically).
- Adding an enum variant makes its exhaustive match sites hub work for the variant's owner.
- Phase 2 writes `berth.toml` at the main worktree root, located through `configuration_lookup` (`ConfigurationLookup::OwnThenMain`); when the main-root file is absent, a validated linked-worktree file's policy (`trunk`, `gate_mode`, `maximum_reservations`, `maximum_ordering_edges`) carries over and the linked file stays; a present main-root file is kept; linked worktrees without their own file read the main-root file, and README "First use" already says so. `read_file` returns `ConfigurationFilePresence::{Missing, Present(BerthConfig)}`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green; the named integration tests pass; the output-contract test passes.
