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

### Phase 3 — Worktree enrollment in `init`  · status: done

#### As-built

- `cargo berth init` (`initialize_ledger` in `cli.rs`) runs `worktree::enroll_worktrees(&WorktreeContext, &BerthConfig) -> Result<WorktreeEnrollmentReport, LedgerError>` after configuration and hooks. A worktree has history when replay holds any `Claim` whose actor worktree is its id, whatever source or lifecycle; such a worktree is never enrolled. Every other live worktree with a non-empty footprint receives one `Claim { source: ClaimSource::Enrolled }` in its own `Ledger::transact` (actor = that worktree, newly issued run), then its run marker via `publish_coordination_run_marker`, so sessions there join the run and reuse the reservation with no append. A re-run enrolls only worktrees still without history and appends nothing else.
- `WorktreeRegistry::enrollment_candidates()` yields `WorktreeEnrollmentCandidate::{Eligible(WorktreeContext), Unavailable { root }}` from the single registry read; locked worktrees enroll, prunable or unavailable ones report `unavailable`.
- Footprint, read outside the lock: operation markers (`rebase-merge`, `rebase-apply`, `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD`) report `operation_in_progress`; HEAD and trunk resolve once via `git rev-parse` (unresolved or unborn reports `no_merge_base`); merge-base exit 1 reports `no_merge_base`; `git::unmerged_branch_paths(trunk, head)` ∪ `observe_merge_working_tree` tracked and untracked paths form exact file scopes. An empty footprint is neither enrolled nor reported. The claim records `phase_start_head` = merge-base, `head_snapshot` and `trunk_at_claim` from the same resolution, and `coordination_identity_provenance: NotPresented`; validation reads no Git.
- Overlaps: `conflicts_for_claim` under `ForeignProtectionPolicy::ExcludeContainedWork(ActingHeadContainment::observe_at_head(..))`, narrowed to shared scopes each side still brings to the other (its working-tree paths or `unmerged_branch_paths(other_head, this_head)`). Pairwise remainders cover candidates and observed worktrees that already have history, so a worktree stacked on an in-flight branch forms no pair for the parent commits it contains. Remaining conflicts become `ConflictAuthorization::Enrollment` via `AuthorizedOverlap::from`, otherwise `NoConflict`; footprints stay trunk-relative.
- `WorktreeEnrollmentReport { enrolled, overlaps, failures }`: enrolled carries reservation id, root, and branch or detached head; `EnrollmentOverlap` carries both reservation ids, shared scopes, and both `cargo berth sequence <a> <b> --why 'Order enrolled work'` commands; failures carry root, reason (`operation_in_progress`, `no_merge_base`, `git_failure`, `unavailable`, `record_too_large`), and diagnostic. Unresolved overlaps replay in `enrollment.rs` from Claim/Widen `Enrollment` authorizations minus matching `ResolveDefer` events, not the ordering graph, so a pair is re-reported after either endpoint ends until `sequence` resolves it. `InitializationPayload.enrollment` is serde-default; the legacy single `init` line prints only when all three lists are empty.
- `RetainedReservationSet` holds `ForeignProtectionPolicy::{FullProtection, ExcludeContainedWork(ActingHeadContainment)}`; `MergeExtent::protected_key()` is removed and its states are matched explicitly. Replayed `ClaimSource::Enrolled` counts as work for automatic ending even after an Empty merge-extent observation, so a trunk enrollment ends once its checkout is clean at a head trunk contains. Enrolled claims skip session-mapping publication, leaving the invoking session unmapped to sibling reservations. `observe_merge_working_tree` omits an untracked `.claude/config/berth.toml` under the observed root (tracked changes still count), so a clean repository enrolls nothing.

**Files:**
- `crates/cargo-berth/src/worktree/enrollment.rs` — enrollment engine, history query, footprint, pairwise containment, per-candidate transaction, run marker, report types, unresolved-overlap replay.
- `crates/cargo-berth/src/worktree/liveness.rs` — `enrollment_candidates()` and `WorktreeEnrollmentCandidate`.
- `crates/cargo-berth/src/worktree/mod.rs` — module and re-exports.
- `crates/cargo-berth/src/reservation/retention.rs`, `containment.rs`, `merge_extent.rs` — `ForeignProtectionPolicy`, `observe_at_head`, explicit extent matching.
- `crates/cargo-berth/src/reconcile.rs` — enrolled source counts as work for automatic ending.
- `crates/cargo-berth/src/session/mod.rs` — no session mapping for enrolled claims.
- `crates/cargo-berth/src/drift/observation.rs` — config-file exclusion in merge observation.
- `crates/cargo-berth/src/cli.rs`, `src/output.rs`, `docs/cargo-berth/generated/output-contract.json` — init wiring and enrollment section.
- `crates/cargo-berth/README.md` — First use: work in flight, overlap report, `sequence`, re-running `init` for a new worktree. `docs/cargo-berth/operations.md` — enrollment section with failure remedies.
- `crates/cargo-berth/tests/ledger.rs`, `tests/gate.rs` — enrollment scenarios and the integration hold for an unresolved pair; `tests/board.rs` — fixtures carry real dirty work instead of the untracked config file.

**Gotchas:**
- The config exclusion lives only in `observe_merge_working_tree`; ordinary and full drift still attribute the untracked config file, and existing tests depend on it.
- `git rev-parse` failures (unborn HEAD, missing trunk branch) surface as `GitError::CommandFailed`, not cat-file resolution variants; they map to `no_merge_base`.
- Containment for existing holders reads their retained merge-extent key, so a holder with a stale saved observation can hide a fresh dirty overlap at enrollment, the same as the edit check.
- A run-marker publish failure after the claim commits is not retried by re-running `init`.
- Managed hooks install into git's hooks directory, so `init` creates no work file beyond the excluded config.

**Ruled out:**
- Refreshing existing holders' merge extents from fresh footprints before binding overlaps (rare; enrollment follows the edit check's containment).
- Retrying run-marker publication for already-enrolled worktrees (requires an obstructed marker path).
- Extending the config-file exclusion to ordinary or full drift.
- A separate enrolled-work evidence field in replay (the immutable `ClaimSource::Enrolled` carries it).

