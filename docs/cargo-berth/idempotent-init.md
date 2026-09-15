# idempotent-init — `cargo berth init` enrolls the worktrees already in flight

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** `init` writes the configuration where every worktree reads it and enrolls every live worktree that has never held a reservation with one reservation covering its footprint since its merge-base with trunk; overlaps between footprints hold integration until answered with `sequence`; re-running `init` is safe and enrolls worktrees added since. Scope (user-set): one user, their own repositories, a handful of linked worktrees, Claude Code sessions as callers — unlikely edge cases get no mechanism.

## Delegation Context

- **Project:** cargo-berth — a reservation engine for git worktrees. `init` sets up the ledger, config, and managed hooks; `claim`, `check`, `drift`, `sequence`, and `integrate` coordinate paths across worktrees. Binary crate: integration tests drive `CARGO_BIN_EXE_cargo-berth`; private APIs are tested with unit tests in the source file.
- **Stack:** Rust edition 2024 (workspace, resolver 3). clap 4.6.6 (derive), serde 1 (derive), serde_json 1, schemars 1, uuid 1 (v7). Dev: cargo-berth-test-support (`GitDriver`, `git_command`), tempfile 3.27.0. Lints: clippy all/pedantic/nursery/cargo deny; `unwrap_used`/`expect_used`/`panic` deny; `missing_docs` deny; `self_named_module_files` deny (a module with children uses `mod.rs`; a leaf module is `name.rs`).
- **Layout:** `crates/cargo-berth/src/`: `cli.rs`, `config.rs`, `output.rs`, `output_contract.rs`, `ledger/` (journal, handle, worktree_context, authorization), `reservation/` (retention, partition), `answer/` (conflict_authorization, scope_binding), `edge/graph.rs`, `verb/` (claim, sequence), `drift/observation.rs`, `session/mod.rs`, `worktree/` (mod, liveness), `git/reachability.rs`, `board/` (rows, answers). Tests: `crates/cargo-berth/tests/`. Docs: `crates/cargo-berth/README.md`, `docs/cargo-berth/operations.md`, `docs/cargo-berth/generated/output-contract.json`.
- **Key files:**
  - `crates/cargo-berth/src/ledger/journal.rs` — `enum JournalOperation` :335; `enum ClaimSource` :559 (`WorkPlan`, `FirstTouch`, `Explicit`); `append_events` :1728.
  - `crates/cargo-berth/src/ledger/handle.rs` — `TransactionValidation` :102; `Ledger::transact` :259 (one operation, actor worktree + run, validated against locked replay); `RecordTooLarge` rejection for records over `MAXIMUM_JOURNAL_RECORD_BYTES` (16 KiB, `src/ledger/constants.rs:19`).
  - `crates/cargo-berth/src/ledger/worktree_context.rs` — `configuration_lookup` :255 (`ConfigurationLookup::OwnThenMain`: a linked worktree reads its own file, then the main worktree's); `publish_coordination_run_marker` :271.
  - `crates/cargo-berth/src/ledger/authorization.rs` — `EditAuthorization::resolve_from_sources` :158 (an unmapped session resolves the worktree's run marker).
  - `crates/cargo-berth/src/config.rs` — `pub(crate) enum Enrollment<T>` :26 (repository configuration presence — unrelated to worktree enrollment, not renamed); `BerthConfig::initialize` :111 (writes to the invoking worktree root); `BerthConfig::read` :142.
  - `crates/cargo-berth/src/cli.rs` — `Init` dispatch :746 → `initialize_ledger` :1425; `SequenceArguments` :562.
  - `crates/cargo-berth/src/answer/conflict_authorization.rs` — `enum ConflictAuthorization` :21 (`NoConflict`, `Sequence`, `Defer`, `Override`, `ExistingAnswersCoverEveryOverlap`); `covers` :99.
  - `crates/cargo-berth/src/answer/scope_binding.rs` — `AuthorizedOverlap` :30 (binds the holder's whole `scope_revision`); `AuthorizedOverlapSet` :42; `AuthorizedOverlap::covers` :84.
  - `crates/cargo-berth/src/reservation/partition.rs` — `reservations_authorize_scope` :155 (checks both reservations' authorizations; used by edit checks and widening).
  - `crates/cargo-berth/src/reservation/retention.rs` — `apply_claim` :1080 (Claim replay sets `MergeExtent::NotDerived { protection: <claim scopes> }` :1101).
  - `crates/cargo-berth/src/edge/graph.rs` — `struct DeferredOverlap` :40; `prepare_deferred_edge` :257; `apply_authorization` :349 (pushes `Defer` deferrals :359/:366); `apply_resolution` :401; `deferred_overlap_between` :459 (either orientation); `AmbiguousDeferral` :484.
  - `crates/cargo-berth/src/verb/sequence.rs` — `execute_sequence` :130 (resolves a pending deferral in either order into an edge).
  - `crates/cargo-berth/src/verb/claim.rs` — `PhaseStartSelection::Protected` :136; `ClaimRepositoryFacts` :139 (private, needs `ClaimRunValidation` — do not use for enrollment); `select_first_touch_reservation_reuse` :1030; `PreparedClaim::into_operation` :1389 (model for building a `Claim` operation).
  - `crates/cargo-berth/src/drift/observation.rs` — `observe_merge_working_tree` :522 (exported; `tracked_paths`, `untracked_paths`).
  - `crates/cargo-berth/src/git/reachability.rs` — `unmerged_branch_paths` :276.
  - `crates/cargo-berth/src/worktree/liveness.rs` — `WorktreeRegistry` :90, impl :139.
  - `crates/cargo-berth/src/session/mod.rs` — `apply_journal_event` :286 (session mapping publication).
  - `crates/cargo-berth/src/output.rs` — `source_description` :4197 (exhaustive `ClaimSource` match); `OutputFacts::Init(InitializationPayload)` :717; `InitializationPayload` :756.
  - `crates/cargo-berth/src/board/answers.rs` — `RecordedAnswer` (`Defer` :42); `AnswerAcquisition` :89. `crates/cargo-berth/src/board/rows.rs` — `UnresolvedOverlap` :272; `unresolved_overlaps` :625.
  - `crates/cargo-berth/src/output_contract.rs` — test `generated_artifacts_are_reproducible_from_the_checked_in_contract` (~:306). `ClaimSource` and authorization kinds appear in the generated schema; regenerate with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1 cargo nextest run -p cargo-berth generated_artifacts_are_reproducible_from_the_checked_in_contract`.
  - Existing behavior the plan relies on: `tests/overlap.rs:183` and `tests/lifecycle.rs:1176` (a session joins a worktree's existing run through its marker and reuses its reservation with no append); `tests/hooks.rs:712` (a worktree added after `init` reads the main config). Worktree helpers: `tests/hooks.rs` `add_worktree` :1570, `tests/edges.rs` :2161, `tests/ledger.rs` :1022.
- **Test lanes:** cargo-berth — `crates/cargo-berth/tests`
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  - The journal is truth; enrollment state is a replay query (plan).
  - Decide and record in one mutation-lock hold; no git inside the lock (repository).
  - Existing overlap answers keep their behavior; the new enrollment variant never revokes edit access (plan).
  - Nothing is auto-removed; an enrolled reservation is retired only by a user action (plan).
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

### Phase 1 — Enrolled claims and enrollment overlap authorization  · status: todo

#### Work Order

**Goal:** The journal can record an enrolled `Claim` whose overlaps with existing reservations are engine-authored deferrals: both sides keep editing, both integrations are held, and `sequence` resolves the pair.

**Spec:**
- **`ClaimSource::Enrolled`** — new variant on `ClaimSource` (`src/ledger/journal.rs:559`). Update `source_description` (`src/output.rs:4197`) and every other exhaustive match; regenerate `output-contract.json`.
- **`ConflictAuthorization::Enrollment { overlaps: AuthorizedOverlapSet }`** — new variant (`src/answer/conflict_authorization.rs:21`); no singular blocker, no reason field, no cause enum. It sits on an enrolled `Claim` whose scopes overlap existing reservations; `overlaps` binds each counterpart and the shared scopes at enrollment.
- **Editing.** `reservations_authorize_scope` (`src/reservation/partition.rs:155`) already checks both reservations' authorizations; reuse it and add no path in `retention.rs`. Implement enrollment coverage in `ConflictAuthorization::covers` (:99): an `Enrollment` authorization covers a scope when the counterpart reservation matches and the scope is among the recorded shared scopes — not the counterpart's whole `scope_revision` (`AuthorizedOverlap::covers`, `scope_binding.rs:84`, keeps revision equality for every existing variant). So an unrelated later widen of either side does not re-block the pair; a new shared path gets ordinary treatment.
- **Integration hold and resolution.** `apply_authorization` (`src/edge/graph.rs:349`) projects one existing `DeferredOverlap` (:40) per counterpart in `overlaps`, with an engine explanation in its existing reason field, exactly as `Defer` does. Existing holds (`IntegrationHold::DeferredOverlap`) and existing `sequence <first> <then> --why <text>` resolution (`prepare_deferred_edge` :257, `apply_resolution` :401, `deferred_overlap_between` :459 in either orientation) apply unchanged. A pair is declared once; three siblings sharing a file yield three pairs across their claims.
- **No incursion.** Later drift in either worktree reports nothing for the recorded overlap, because each side is covered by its own reservation and the authorization.
- **Presentation.** `src/board/answers.rs` (`RecordedAnswer`, `AnswerAcquisition` :89) and `src/board/rows.rs` (`unresolved_overlaps` :625) label enrollment-authored deferrals as enrollment, not as a user answer.
- **Tests are unit tests** in the source files (the builders are private): replay of an enrolled `Claim` with `Enrollment` authorization projects a deferral per counterpart; `ConflictAuthorization::covers` accepts shared scopes after an unrelated counterpart widen and rejects a new shared scope; `apply_resolution` resolves an enrollment deferral given either order.

**Files:**
- `crates/cargo-berth/src/ledger/journal.rs` — `ClaimSource::Enrolled`.
- `crates/cargo-berth/src/answer/conflict_authorization.rs` — `Enrollment` variant and its `covers` rule; unit tests.
- `crates/cargo-berth/src/answer/scope_binding.rs` — shared-scope matching helper if `covers` needs one.
- `crates/cargo-berth/src/edge/graph.rs` — project `Enrollment` into `DeferredOverlap`; unit tests.
- `crates/cargo-berth/src/output.rs` — `source_description` and exhaustive matches.
- `crates/cargo-berth/src/board/answers.rs` — enrollment label.
- `crates/cargo-berth/src/board/rows.rs` — enrollment label on unresolved overlaps.
- `docs/cargo-berth/generated/output-contract.json` — regenerated.

**Seats:** 3 writers — authorization/graph, journal/output/contract, and board split by file; tests are unit tests owned by each writer (no command creates enrolled claims until Phase 3).
- `impl` — `src/answer/conflict_authorization.rs`, `src/answer/scope_binding.rs`, `src/edge/graph.rs`; hub: `src/answer/conflict_authorization.rs` (the variant every other file matches on).
- `test` — opens as impl: `src/ledger/journal.rs`, `src/output.rs`, `docs/cargo-berth/generated/output-contract.json`.
- `review` — opens as impl: `src/board/answers.rs`, `src/board/rows.rs`.

**Constraints from prior phases:** None.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green; the three unit tests pass; the output-contract and `reader_compat` tests pass inside the full test run.

### Phase 2 — Configuration at the main worktree root  · status: todo

#### Work Order

**Goal:** `cargo berth init` in any worktree writes `berth.toml` at the main worktree root, so every worktree reads one configuration.

**Spec:**
- **Config.** `init` writes `berth.toml` at the main worktree root, taken from the worktree's existing `configuration_lookup` (`ConfigurationLookup::OwnThenMain`, `src/ledger/worktree_context.rs:255`), not the invoking root (`BerthConfig::initialize`, `src/config.rs:111`). A present main-root file is kept. If absent and the invoking linked worktree has its own validated `berth.toml`, the main file carries that file's policy (trunk, `gate_mode`, limits); otherwise defaults. The linked file is left in place. No comparison or removable-file reporting.
- **Caller.** `LedgerHandle::initialize` (`src/ledger/handle.rs`) passes the main root to `BerthConfig::initialize`. Wire `initialize_ledger` (`src/cli.rs:1425`) only if the main root must reach it from there.
- **Output.** No report fields and no output-contract change (author scope note: this phase adds nothing to `InitializationPayload`).
- **Docs.** One line in `crates/cargo-berth/README.md` "First use": the configuration lives at the main worktree root, and every linked worktree reads it.

**Files:**
- `crates/cargo-berth/src/config.rs` — main-root target and linked-file carry-over.
- `crates/cargo-berth/src/ledger/handle.rs` — `LedgerHandle::initialize` passes the main root.
- `crates/cargo-berth/src/cli.rs` — `initialize_ledger` :1425, only if wiring is needed.
- `crates/cargo-berth/README.md` — one First use line.
- `crates/cargo-berth/tests/hooks.rs` — integration tests (helper `add_worktree` :1570; `a_worktree_added_after_init_coordinates_without_its_own_configuration` :712 is the existing model).

**Seats:** 1 writer + 1 tester + reserve — one change path: the config target, its caller, and the one doc line do not split.
- `impl` — `src/config.rs`, `src/ledger/handle.rs`, `src/cli.rs`, `crates/cargo-berth/README.md`; hub: `src/config.rs` (the initialize signature the caller follows).
- `test` — in `tests/hooks.rs`: `init_from_a_linked_worktree_writes_the_main_root_configuration` (`init` run from a linked worktree writes the main-root file, and a sibling linked worktree with no file of its own reads it); `init_carries_a_linked_configuration_policy_to_the_main_root` (a linked-only config's trunk and `gate_mode` carry over); `init_keeps_a_present_main_root_configuration` (a present main-root file is kept unchanged).
- `review` — reserve.

**Constraints from prior phases:** Phase 1 added `ClaimSource::Enrolled` and `ConflictAuthorization::Enrollment { overlaps: AuthorizedOverlapSet }`, whose coverage matches counterpart plus recorded shared scopes; replay projects one `DeferredOverlap` per counterpart, so existing integration holds and `sequence` resolution apply; board rows label enrollment deferrals.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green; `init_from_a_linked_worktree_writes_the_main_root_configuration`, `init_carries_a_linked_configuration_policy_to_the_main_root`, and `init_keeps_a_present_main_root_configuration` pass.

### Phase 3 — Worktree enrollment in `init`  · status: todo

#### Work Order

**Goal:** `cargo berth init` enrolls every never-reserved live worktree with one reservation per worktree; a second run appends nothing new.

**Spec:**
- **Module.** New leaf module `src/worktree/enrollment.rs` (`mod enrollment;` in `src/worktree/mod.rs`).
- **History query.** A worktree has reservation history when replay holds any `Claim` whose actor worktree is its id, whatever source or lifecycle. Such a worktree is never enrolled.
- **Candidates.** Read `WorktreeRegistry` (`src/worktree/liveness.rs:90`) once. A candidate has registration `Available` or `Locked` and location `Discovered`; a `Prunable` or `Unavailable` worktree is reported as a failure.
- **Footprint**, per candidate, outside the lock:
  1. Resolve trunk and `HEAD` object ids once.
  2. If `rebase-merge`, `rebase-apply`, `MERGE_HEAD`, `CHERRY_PICK_HEAD`, or `REVERT_HEAD` exists in that worktree's git dir, report `operation_in_progress`.
  3. Merge-base of trunk and `HEAD`; none, trunk unresolved, or unborn `HEAD` reports `no_merge_base`.
  4. Committed: `git::unmerged_branch_paths(trunk, head)` (`src/git/reachability.rs:276`).
  5. Working tree: `drift::observe_merge_working_tree` (`src/drift/observation.rs:522`) `tracked_paths` ∪ `untracked_paths`, in that worktree's root.
  6. Union as exact file scopes. Empty: not enrolled, not reported.

  A failing git command reports `git_failure` with the command and error; other candidates proceed.
- **Enrollment transaction**, one `Ledger::transact` (`src/ledger/handle.rs:259`) per candidate, actor = candidate worktree id and a newly issued run. Inside validation: skip if the locked replay shows history; compute overlaps against current reservations (earlier candidates' claims included); append one `Claim` with `source: ClaimSource::Enrolled`, `purpose` naming enrollment and the branch or detached head, `phase_start_head` = merge-base (`PhaseStartSelection::Protected`), `head_snapshot` and `trunk_at_claim` from step 1, `worktree_root` and `worktree_administrative_locator` from the candidate's `WorktreeContext`, `coordination_identity_provenance: NotPresented`, `authorization` = `NoConflict` or Phase 1's `Enrollment { overlaps }`. Build the operation directly, modeled on `PreparedClaim::into_operation` (`src/verb/claim.rs:1389`); do not use `ClaimRepositoryFacts`. A `RecordTooLarge` rejection reports `record_too_large` for that worktree. Session mapping publication (`session::apply_journal_event`, `src/session/mod.rs:286`) is skipped for enrolled claims so the invoking session is not mapped to sibling reservations.
- **Run marker.** After each claim succeeds, publish its run with `publish_coordination_run_marker` (`worktree_context.rs:271`) in that worktree. Sessions there then join the run through existing identity resolution and reuse the reservation with no append (`tests/overlap.rs:183`).
- **Output.** Additive enrollment section on `InitializationPayload` (`src/output.rs:756`); regenerate `output-contract.json`:
  - enrolled: reservation id, worktree root, branch or detached head;
  - overlaps: both reservation ids, shared scopes, and the two ready-to-run `sequence <a> <b> --why <text>` commands (both orders);
  - failures: worktree root, reason (`operation_in_progress`, `no_merge_base`, `git_failure`, `unavailable`, `record_too_large`), and diagnostic.
  - Worktrees with history or empty footprints are omitted. The single legacy `init` line remains when nothing enrolled and nothing failed.
- **Idempotence.** A re-run enrolls only worktrees still without history (added since, or previously failed) and re-reports unresolved overlaps.
- **Docs.** `crates/cargo-berth/README.md` "First use": `init` on a repository with work in flight, reading the overlap report, answering with `sequence`, and re-running `init` after adding a worktree that already has commits, before its first edit. `docs/cargo-berth/operations.md`: a short enrollment section (config location, failure reasons and what to do).

**Files:**
- `crates/cargo-berth/src/worktree/mod.rs` — `mod enrollment;`.
- `crates/cargo-berth/src/worktree/enrollment.rs` — new: history query, candidates, footprint, per-candidate transaction, run marker, outcomes.
- `crates/cargo-berth/src/session/mod.rs` — skip mapping publication for `ClaimSource::Enrolled`.
- `crates/cargo-berth/src/cli.rs` — `initialize_ledger` :1425 runs enrollment after config and hooks.
- `crates/cargo-berth/src/output.rs` — enrollment section.
- `docs/cargo-berth/generated/output-contract.json` — regenerated.
- `crates/cargo-berth/README.md` — First use.
- `docs/cargo-berth/operations.md` — enrollment section.
- `crates/cargo-berth/tests/ledger.rs` — integration tests.
- `crates/cargo-berth/tests/gate.rs` — integration tests.

**Seats:** 2 writers + 1 tester — enrollment engine split from CLI/output/docs.
- `impl` — `src/worktree/mod.rs`, `src/worktree/enrollment.rs`, `src/session/mod.rs`; hub: `src/worktree/enrollment.rs` (the outcome type output renders — define `EnrollmentOutcome` first).
- `test` — in `tests/ledger.rs`, one three-worktree scenario (two with committed and dirty edits to one shared file, one on trunk with dirty edits) run with `init` from a linked worktree: three reservations; overlap reported with two `sequence` commands, and one of them runs as rendered and resolves it; second `init` appends nothing; a worktree whose reservation was released is not re-enrolled; a worktree mid-rebase reports `operation_in_progress`; a worktree added after `init` with commits enrolls on the next `init`. In `tests/gate.rs`: the unresolved enrollment pair holds integration (reported under observe, rejected under enforce), and both sides edit the shared file with no incursion.
- `review` — opens as impl: `src/cli.rs`, `src/output.rs`, `docs/cargo-berth/generated/output-contract.json`, `crates/cargo-berth/README.md`, `docs/cargo-berth/operations.md`.

**Constraints from prior phases:** Phase 1 added `ClaimSource::Enrolled` and `ConflictAuthorization::Enrollment { overlaps: AuthorizedOverlapSet }`, whose coverage matches counterpart plus recorded shared scopes; replay projects one `DeferredOverlap` per counterpart, so existing integration holds and `sequence` resolution apply; board rows label enrollment deferrals. Phase 2 writes `berth.toml` at the main worktree root, located through `configuration_lookup` (`ConfigurationLookup::OwnThenMain`); when the main-root file is absent, a validated linked-worktree file's policy (trunk, `gate_mode`, limits) carries over and the linked file stays; a present main-root file is kept; every worktree reads the main-root file, and README "First use" already carries a line saying so.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`, `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` green; the named integration tests pass; the output-contract test passes.
