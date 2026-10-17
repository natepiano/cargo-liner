# berth-next

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Initializing berth leaves the trunk worktree clean, the terminal board shows reservations created after it opens and shows no raw diagnostics, and a single-reservation board answer carries that reservation's ordering edges and overlap answers.

> **As-built disposition: amend** — `docs/cargo-berth/as-built/worktree-enrollment.md`, `docs/cargo-berth/as-built/parallel-phases.md`, `docs/cargo-berth/as-built/worktree-coordination.md`, `docs/cargo-berth/as-built/json-contract.md`, `docs/cargo-berth/as-built/operations.md`, `docs/cargo-berth/as-built/rebase-settlement.md`

> **Production: remove-running-section** — unit `berth-parallel-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, from the showrunner: the three berth-parallel next items (that file is retired; each item is a phase here), one phase each. Phase 2 was added the same day from organon's berth report (resolved incursions re-raised for commits already on `init/catalyst`); the showrunner placed it ahead of the board phases because it hits every session's hook. A second case came from cables-unit the same day (merge e540ed18d of `init/catalyst`, then a plan-doc edit; reservation 01a14753 charged with incursions for files the merge only received). Phase 3 (a trial merge read as an edit, from handler-enh) and phase 4 (claiming a new path behind an existing edge, from the showrunner) were added that evening, ahead of the board phases. They are berth defects under the user's berth standing approval: fast fix with tests, install on both machines after each checkpoint.

## Delegation Context

- **Project:** `cargo-berth`, the worktree reservation ledger and hook engine.
- **Project started:** 2026-10-16T23:47:40.606+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`. Every seat works only here, and every Work Order names it.
- **Base commit:** `b2d6f65d` (main).
- **Stack:** Rust workspace, cargo-nextest, ratatui + crossterm for the terminal board. The crate has no lib target, so unit tests live in `cargo-berth::bin/cargo-berth` and integration tests in `crates/cargo-berth/tests/`.
- **Test lanes:** `crates/cargo-berth/tests/`.
- **Key files:**
  - `crates/cargo-berth/src/config.rs` — `BerthConfig::relative_path` (145), `BerthConfig::exclude_from_git` (155), `BerthConfig::initialize` (187), `CLAUDE_DIRECTORY` / `CONFIGURATION_DIRECTORY` / `CONFIGURATION_FILE` (17–19).
  - `crates/cargo-berth/src/ledger/handle.rs` — `Ledger::initialize` (~256–267) calls `BerthConfig::initialize`.
  - `crates/cargo-berth/src/cli.rs` — `initialize_ledger` (~1711): ledger init, `pin_unrecorded_targets`, `gate::install::install_managed_hooks`, `worktree::enroll_worktrees`.
  - `crates/cargo-berth/src/drift/observation.rs` — `observe_merge_working_tree` (~560–583) drops `BerthConfig::relative_path()` from the merge working tree.
  - `crates/cargo-berth/src/board/tui.rs` — `run(model: &BoardModel)` (~106), `BoardApplication::new` (~502), `BoardPaneDocuments::from_model` (~251, Overview text at ~266–270), `OVERVIEW_FIELDS` (~64–70), `overview_reservation_table` (~337), `event_loop` (~639–666, blocking `event::read()`, no tick), unit tests `mod tests` (~1094, `TestBackend`), helper `reassembled_model_value` (~319, ~1368) splits Overview text at the first `"\n\n"`.
  - `crates/cargo-berth/src/verb/board.rs` — `execute` (47) reconciles and calls `execute_complete_board` (~85: `BoardModel::build` at 90, `tui::run(&board)` at 113); `BoardOutputSelection::ReservationLifecycleFor` (29) → `execute_reservation_lifecycle` (~137).
  - `crates/cargo-berth/src/board/rows.rs` — `BoardModel` (~85–104), `BoardModel::build` (~597; edges via `declared_ordering_constraints(&report.constraints, &active_ids)` and `unresolved_overlaps(&report.constraints)` ~633–634), `WaitingEntry` / `WaitingHold` (~356), `SettledOrderingConstraint` (~416), `BoardJournalPosition` (~150).
  - `crates/cargo-berth/src/board/answers.rs` — `RecordedOverlapAnswer` (~43), `board_overlap_answers`.
  - `crates/cargo-berth/src/board/report.rs` — `reservation_lifecycle_snapshot(report, id)` (79), `ReservationReport` (17, 50), `reservation_lifecycle_presentation`.
  - `crates/cargo-berth/src/output.rs` — `OutputFacts::Reservation(ReservationLifecycleQueryPayload)` (~762, payload kind `"reservation"`), `ReservationLifecycleQueryPayload` (~1049), `ReservationReportSnapshot` (~1058: `reservation_id`, `lifecycle`, `race_extent`, `merge_extent`), `OutputEnvelope::reservation_lifecycle` (~1748).
  - `crates/cargo-berth/src/output_contract.rs` — schemas `reservation_lifecycle` / `reservation_lifecycle_query` (~117–124); regenerate `docs/cargo-berth/generated/output-contract.json` with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1` (test at ~312).
  - `crates/cargo-berth/tests/ledger.rs` — init tests; `clean_init_ignores_its_untracked_configuration_and_keeps_the_legacy_line` asserts a clean `git status` after `init`; `CONFIGURATION_PATH`, `EXCLUDE_PATH`.
  - `crates/cargo-berth/tests/board.rs` — `reservation_report` / `reservation_report_and_presentation` helpers (~2039–2044), `waiting_successor_lifecycle_and_extents_are_queryable_and_listed_with_its_hold` (614).
  - `crates/cargo-berth/README.md` — "First use" (10, init at 28–57), "Board" (382–420).
  - `crates/cargo-berth/CHANGELOG.md` — `[Unreleased]` `Fixed` (~87).
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Invariants:**
  - Journal replay of existing ledgers is unchanged.
  - Invalid states are unrepresentable: fix with types, never procedural guards (user, 2026-10-15).
  - No test takes a second. Format with `cargo +nightly fmt` only. Run one `verify.sh` at a time.
  - `crates/cargo-berth/CHANGELOG.md` and `crates/cargo-berth/README.md` are shared by every phase, so the phases run in order.
- **Constraints from prior phases (every remaining Work Order):**
  - `init` adds `/.claude/config/berth.toml` to the common git `info/exclude`, so the configuration is ignored in every worktree. A fixture that commits the configuration on purpose uses `git add --force`; a fixture standing for a repository initialized before that change removes the exclude entry.
  - A fixture that needs a dirty tree or uncommitted work creates real work files; untracked configuration no longer counts as work.
  - The target's locally available upstream (`LocalTargetUpstream`, `src/git/refs.rs`) filters incursion authorship only. Release, settlement and integration judge the local target branch; no change widens upstream reachability to them.
  - An incident journalled before phase 2 keeps holder-subset coverage (`IncursionCoverageEvidence`, `src/ledger/journal.rs`); only incidents journalled since carry commit-keyed coverage.
  - A working-tree incursion answer covers the single commit that records that write; a further commit entering the path reopens the incident (`EnteringWriteProgress`).

## Phases

### Phase 1 — Initializing berth leaves no untracked `.claude/` in trunk  · status: done

#### As-built

- `BerthConfig::exclude_from_git(common_git_directory: &Path) -> Result<(), ConfigError>` appends `/.claude/config/berth.toml` once to the common git `info/exclude`. The line is built from `BerthConfig::relative_path()`, so the excluded path and the written path cannot diverge. It creates `info/` when absent, keeps existing bytes (adding a newline separator when the file lacks a trailing one), and treats an anchored or unanchored line, with or without CRLF, as already present.
- `initialize_ledger` calls it on every `init`, right after worktree-context discovery and before the configuration read, so rerunning `init` repairs a repository initialized before the exclude existed. A write failure returns `initialization_error(LedgerError::Config(..))` and `init` exits non-zero; rerunning `init` after removing the obstruction writes the line once.
- The configuration path and contents are unchanged; `observe_merge_working_tree` still drops `BerthConfig::relative_path()`. A tracked configuration stays visible, since exclude never hides a tracked file.
- README "First use", CHANGELOG `[Unreleased]` `Fixed`, and the as-built `worktree-enrollment.md` and `operations.md` state the exclude.

**Files:**
- `crates/cargo-berth/src/config.rs` — exclude path ownership and the write
- `crates/cargo-berth/src/cli.rs` — `initialize_ledger` calls the write on every `init`
- `crates/cargo-berth/tests/ledger.rs` — clean init leaves nothing under `.claude/`, repeat init keeps one line, linked worktree is clean, existing patterns survive, missing `info/` is created, tracked configuration stays visible
- `crates/cargo-berth-test-support/src/integration_repository.rs` and the integration test files that commit the configuration — `git add --force`
- `crates/cargo-berth/README.md`, `crates/cargo-berth/CHANGELOG.md`, `docs/cargo-berth/as-built/worktree-enrollment.md`, `docs/cargo-berth/as-built/operations.md`

**Binds later work:** a fixture that commits `.claude/config/berth.toml` uses `git add --force`; a legacy-repository fixture removes `info/exclude`; a fixture that needs a dirty tree creates real work files, never relying on the configuration. The exclude-write failure and rerun repair is covered by the incursion phase's test (h).

**Gotchas:**
- A later `!` negation line in the exclude file is honored as the user's un-ignore.
- Before this change the untracked configuration acted as implicit dirty work in several fixtures (drift root-head, engine instructions); those now create real work files.

**Ruled out:** serializing concurrent `init` exclude writes — single-user race.

### Phase 2 — An incursion from commits already on the integration target is never raised, and a resolved one stays resolved  · status: done

**Depends on:** Phase 1 (shared README and CHANGELOG)

#### As-built

- Incursion classification drops every in-range commit the integration target reaches, locally or through its upstream ref already present locally (never fetched), so a path whose in-range commits are all on the target raises no incursion. The committed-history read covers every committed path, including one that is also dirty. `IntegrationTarget::local_upstream` returns `LocalTargetUpstream`; no upstream configured, or upstream objects absent, falls back to the local target alone. Release, settlement and integration still judge the local target.
- `CommittedForeignPathHistory` (`NotRead` / `NoCommittedPaths` / `Unavailable` / `Read(IncursionAttributionBatch)`) carries the history into classification; `ForeignPathWriteOrigin` and `IncursionPathObservation` give each path its write origin.
- Each journal `BlockedIncursionPath` records `coverage: IncursionCoverageEvidence` — `LegacyHolderCoverage`, `KnownEnteringCommits { commits: EnteringCommitSet }`, `WorkingTreeWrites`, `HistoryUnavailable`. A resolved incident stays resolved when its holder set grows on unchanged commits; a further entering commit reopens it. A working-tree answer also covers the single commit that records that write (`EnteringWriteProgress`); two or more entering commits reopen it. Both earlier journal layouts replay with `LegacyHolderCoverage`.
- Hook STOP lines for received commits and for answered writes no longer appear.
- CHANGELOG `[Unreleased]` `Fixed`, `worktree-coordination.md`, `parallel-phases.md` and `json-contract.md` (journal `coverage` layout) state the behavior.

**Files:**
- `crates/cargo-berth/src/drift/classification.rs` — `ForeignPathWriteOrigin`, `IncursionPathObservation`, coverage evidence per path
- `crates/cargo-berth/src/drift/provenance.rs` — `CommittedForeignPathHistory`, `path_commit_authorship`, `read_committed_foreign_paths`
- `crates/cargo-berth/src/git/refs.rs`, `reachability.rs`, `constants.rs` — `LocalTargetUpstream`, upstream exclusion with local fallback, `GIT_UPSTREAM_SELECTOR_SUFFIX`
- `crates/cargo-berth/src/ledger/target.rs` — `IntegrationTarget::local_upstream`
- `crates/cargo-berth/src/ledger/journal.rs` — `IncursionCoverageEvidence`, `EnteringCommitSet`, `EnteringWriteProgress`, legacy-layout replay
- `crates/cargo-berth/src/reservation/retention.rs` — commit-keyed `incursion_path_coverage`
- `crates/cargo-berth/tests/drift.rs`, `tests/hooks.rs`, `tests/ledger.rs` (exclude-write failure and rerun repair), `tests/gate.rs`
- `crates/cargo-berth/CHANGELOG.md`, `docs/cargo-berth/as-built/worktree-coordination.md`, `parallel-phases.md`, `json-contract.md`

**Binds later work:** the trial-merge phase defers before `read_committed_foreign_paths` and the upstream lookup run, so neither happens mid-merge.

**Gotchas:**
- Git's upstream selector takes the short branch name; `refs/heads/<b>@{upstream}` fails with "no such branch".
- Gate process-cardinality budgets in `tests/gate.rs` count one upstream lookup per distinct target (17).

**Ruled out:** treating merge history simplification as hiding an authored edit — git prunes a parent only when the merge result matches the other parent for that path, so the pruned edit is not in the result.

### Phase 3 — A trial merge in progress is never read as this worktree's edit  · status: done

**Depends on:** Phase 2 (shared CHANGELOG, `src/drift/`, as-built docs)

#### As-built

- `WorktreeOperation` (`pub(crate)`: `Idle` / `Rebase` / `Merge` / `CherryPick` / `Revert`) and `WorktreeOperation::observe(worktree_administrative_directory: &Path) -> Self` read one marker set — `rebase-merge`, `rebase-apply`, `MERGE_HEAD`, `CHERRY_PICK_HEAD`, `REVERT_HEAD` — whose path constants live in `src/git/constants.rs`. A rebase marker wins when a stopped replay also exposes another operation's marker. It replaces `git::rewrite_in_progress` and enrollment's `OPERATION_IN_PROGRESS_MARKERS`; drift, enrollment and rewrite reconciliation all read it.
- Drift: `comparable_worktree` returns `WorktreeComparability::DeferredPendingOperation` for any operation other than `Idle`, and `prepare_worktree_comparison` turns it into the unchanged empty report. The decision runs inside `prepare_drift_execution` before fingerprint observation, the upstream lookup and `read_committed_foreign_paths`, so a pending operation creates no first touch, no incursion and no fingerprint cache entry. After `--abort` the clean tree reports nothing; a completed merge commit is judged by the committed-origin incursion rules.
- Enrollment: `check_enrollment_operation` rejects with `WorktreeEnrollmentFailureReason::OperationInProgress` and a diagnostic in words ("a merge is in progress"); the wire reason is unchanged.
- Rewrite reconciliation: `prepare_rewrite_reconciliation` skips a branch-rewrite marker only when `observe` returns `Rebase`, so a pending merge, cherry-pick or revert does not stop reconciliation.
- CHANGELOG `[Unreleased]` `Fixed`, `worktree-coordination.md`, `rebase-settlement.md` and `worktree-enrollment.md` state the deferral and that a completed `--no-commit` cherry-pick or revert reads as this worktree's edit.

**Files:**
- `crates/cargo-berth/src/git/worktree_operation.rs` — `WorktreeOperation`, the marker set, `observe` and its unit test
- `crates/cargo-berth/src/git/constants.rs`, `src/git/mod.rs`, `src/git/discovery.rs` — marker path constants, re-export, `rewrite_in_progress` removed
- `crates/cargo-berth/src/drift/execution.rs` — `DeferredPendingOperation` and `drift_reports_no_change_while_a_git_operation_is_pending`
- `crates/cargo-berth/src/worktree/enrollment.rs`, `src/worktree/constants.rs` — `check_enrollment_operation` and `enrollment_names_pending_operations_in_words`
- `crates/cargo-berth/src/reconcile.rs` — rebase-only deferral through the shared type
- `crates/cargo-berth/tests/hooks.rs` — trial merge unreserved and its abort clear, trial merge not charged to an active reservation, cherry-pick stopped on a real conflict, completed peer merge judged after a cached clean check
- `crates/cargo-berth/CHANGELOG.md`, `docs/cargo-berth/as-built/worktree-coordination.md`, `rebase-settlement.md`, `worktree-enrollment.md`

**Gotchas:**
- A completed `--no-commit` cherry-pick or revert leaves no marker and reads as an ordinary dirty tree, by design: git records no pending operation.
- The integration tests reach each state through real git commands (a conflicting cherry-pick writes `CHERRY_PICK_HEAD`), never a written marker file.

**Ruled out:** separate cache invalidation for a completed merge — git unlinks `MERGE_HEAD` before `post-commit` runs, so the installed hook judges the merge commit in full even after a cached clean fingerprint.
### Phase 4 — Claiming a new path behind a holder you already sequenced against succeeds  · status: done

**Depends on:** Phase 3 (shared CHANGELOG and as-built docs)

#### As-built

- `claim <path> --reservation R --before H` (or `--after H`) reuses R's existing edge in that direction: it adds the path, keeps the edge's identity and original reason, and exits 0. `ClaimRejection`/`ClaimError::DuplicateOrdering` and `OutputEnvelope::claim_duplicate_ordering` are gone; `OutputStatus::DuplicateOrderingEdge` remains for the `sequence` verb only. The opposite direction is still refused (it needs `--replace`); `--defer` is unchanged.
- `OrderingGraph::bind_claim_sequence(requester, SequencedBlocker, &OverlapAuthorizationReason) -> ClaimSequenceBinding` (replaces `contains_sequence`) returns `ClaimSequenceBinding::NewEdge { holder, reason }` or `ExistingEdge { holder, reason }`, the latter carrying the recorded edge's `EdgeId` and reason. `ConflictAuthorization::sequence_with_bindings` records `ConflictAuthorization::SequenceWithExistingEdges { overlaps, holders: Vec<ClaimSequenceBinding>, replaced }` when any binding is `ExistingEdge`, else the plain `Sequence` / `SequencePerHolder` forms. Only `NewEdge` bindings count toward the edge limit.
- Replay (`apply_claim_sequence_bindings`) adds an edge for each `NewEdge` and widens the scopes of each `ExistingEdge`; a binding whose id, endpoints or reason do not match a live edge fails with `EdgeReplayError::InvalidExistingEdge(EdgeId)`. Existing ledgers replay unchanged.
- A repeat claim whose paths the existing edge's recorded `Claim`/`Widen`/`Answer` authorizations already cover (`ConflictAuthorization::covers_ordering_edge`) returns `CommittedActionValidation::AlreadySatisfied` → `LedgerCommittedActionOutcome::AlreadySatisfied`: nothing is appended to the journal, the acquisition reports `AlreadyHeld`, and the session selection of R is still published. The reconciliation path returns its own `LedgerReconciliationOutcome { Appended, Rejected }`, which cannot be `AlreadySatisfied`.
- Board: `RecordedOverlapAnswer::Sequence` carries `edge_id` for every sequence form; `SequenceWithExistingEdges` renders one `sequence` answer per holder with that holder's edge identity, direction and recorded reason, naming the new scopes. `docs/cargo-berth/generated/output-contract.json` carries the field.

**Files:**
- `crates/cargo-berth/src/answer/conflict_authorization.rs`, `src/answer/mod.rs` — `ClaimSequenceBinding`, `SequenceWithExistingEdges`, `sequence_with_bindings`, `covers_ordering_edge`
- `crates/cargo-berth/src/edge/graph.rs` — `bind_claim_sequence`, `apply_claim_sequence_bindings`, `InvalidExistingEdge`, and the identity-and-relationship replay test
- `crates/cargo-berth/src/verb/claim/mod.rs`, `src/verb/claim/named.rs` — binding per holder, new-edge count, `NamedClaimMutation`, `ordering_is_already_satisfied`
- `crates/cargo-berth/src/ledger/handle.rs`, `src/ledger/journal.rs`, `src/ledger/mod.rs` — the `AlreadySatisfied` outcomes, `LedgerReconciliationOutcome`, widen classification
- `crates/cargo-berth/src/board/answers.rs` — `append_holder_sequence_answers` and `edge_id` on sequence answers
- `crates/cargo-berth/src/output.rs` — claim-only duplicate envelope removed
- `crates/cargo-berth/tests/parallel_phases.rs` — before/after reuse with journal-free repeats, two holders with one new edge, opposite direction refused, widen-then-replay
- `crates/cargo-berth/tests/board.rs` — `widening_under_an_existing_{before,after}_edge_lists_its_new_scope_answer`
- `crates/cargo-berth/CHANGELOG.md`, `docs/cargo-berth/as-built/json-contract.md`, `worktree-coordination.md`, `phase-reservations.md`, `docs/cargo-berth/generated/output-contract.json`

**Binds later work:** `RecordedOverlapAnswer::Sequence.edge_id` links each sequence answer to its ordering edge, and a `SequenceWithExistingEdges` authorization appears as `sequence` answers bound to an existing edge; the single-reservation board answer ("A single-reservation board answer carries its ordering edges and overlap answers") lists both.

**Gotchas:** without `--reservation`, a repeated direction still records a second edge; the reuse applies only to a named reservation.

**Ruled out:** `ExistingAnswersCoverEveryOverlap` for the widen — that form means earlier answers already authorize every scope and records no board answer; an optional edge on the plain `Sequence` form.

### Phase 5 — The terminal board shows reservations created after it opens, and its Overview holds no raw diagnostics  · status: todo

**Depends on:** Phase 4 (shared README and CHANGELOG)

#### Work Order

**Goal:** A reservation created while the board is open appears in the Overview and Reservations panes without a restart, and the Overview shows only its Reservations table.

**Spec:**
1. `event_loop` waits with `event::poll` up to the next refresh deadline instead of a blocking `event::read()`. Refresh checks run once a second against a monotonic deadline, whether or not events arrive, so sustained key or resize input never starves them. Each check asks whether the ledger moved (the journal position the model was built at against the ledger's current one) with a read that does not reconcile; only when it moved does it rebuild: re-run the same `reconcile::reconcile` + `BoardModel::build` path `verb::board::execute` uses, rebuild the pane documents, and redraw. An unchanged ledger costs no reconcile. `ValidatedJournal` (`src/ledger/handle.rs:80`) keeps no generation or offset and the projection's position fields are private to the ledger, so add a read-only ledger API that returns the current journal position as a semantic type the board compares with the one its model was built at.
2. `tui::run` takes a model source instead of one `&BoardModel`: a type owned by `verb/board.rs` (or `board/`) that holds the invocation directory and builds a fresh `BoardModel`. A rebuild that fails keeps the last good model and its journal position on screen, so the next check retries, and shows the failure in the status line until the next good rebuild; it never exits the board.
3. A rebuild keeps the selected pane and each pane's scroll offset, clamped to the new document length.
4. The Overview pane shows only the Reservations table. The five `OVERVIEW_FIELDS` (`journal_position`, `recovered_bypasses_this_invocation`, `integration_order`, `targets`, `git_cost`) leave the terminal board entirely, with no new pane: `board --json` already carries them for anyone diagnosing (showrunner, 2026-10-16: fewer things on screen wins). Each `BoardSection` also serializes its own `journal_position` (`src/board/rows.rs:161-163`); drop that nested field from every pane document too. `BoardPaneDocuments::from_model` drops these fields by name instead of failing on them as unassigned, so every other model field still has exactly one pane, and `board --json` output is unchanged. Update `reassembled_model_value` (no Overview split at `"\n\n"`) and `every_model_fact_is_reachable_through_exact_pane_documents` (every model field except the five is reachable), and retire `recovered_bypasses_render_only_in_the_model_that_reports_them`, whose fact the board no longer renders, unless `board --json` lacks a test for it.
5. Tests (unit tests in `tui.rs`, `TestBackend`): (a) a model rebuilt with one more reservation renders that reservation in the Overview and Reservations panes; (b) no pane's text contains any of the five fields (`git_cost`, `journal_position`, …), nested section positions included; (c) every other model field is reachable through exactly one pane; (d) a failed rebuild keeps the previous rows and position, shows the failure, and the next check retries; (e) selection and scroll survive a rebuild, clamped; (f) a check with an unchanged position runs no reconcile; (g) sustained input with no idle gap still refreshes by the deadline. The loop reads events and the clock through a trait the tests implement, so no test sleeps.
6. Docs: README "Board" (~388–427) states the live refresh and that the raw diagnostics are in `board --json`, not the terminal board; CHANGELOG `Fixed` line; `docs/cargo-berth/as-built/parallel-phases.md` ("Board Overview Reservations table" ~102–114, its Overview tests ~144–145, and the known-defect lines ~157–158) and `worktree-coordination.md` (~441) describe the refresh and the Overview that shows only the table.

**Files:**
- `crates/cargo-berth/src/board/tui.rs` — event loop, refresh, panes, unit tests
- `crates/cargo-berth/src/verb/board.rs` — model source passed to `tui::run`
- `crates/cargo-berth/src/board/rows.rs` — the model's journal position for comparison
- `crates/cargo-berth/src/ledger/handle.rs`, `crates/cargo-berth/src/ledger/projection.rs` — read-only current journal position
- `crates/cargo-berth/README.md`, `crates/cargo-berth/CHANGELOG.md`
- `docs/cargo-berth/as-built/parallel-phases.md`, `docs/cargo-berth/as-built/worktree-coordination.md`

**Workers:** 1 writer + 1 tester — nothing splits: the tests are unit tests inside `tui.rs`, so `impl` takes every file.
- `impl` — every file above, the Spec 5 tests included; runs lint and the full gate.
- `test` — opens as reviewer of the Spec 5 tests once `impl` posts them; writes nothing.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; a live board on natedev shows a reservation created after it opened within two seconds, and its Overview shows only the table.

### Phase 6 — A single-reservation board answer carries its ordering edges and overlap answers  · status: todo

**Depends on:** Phase 5 (shared README and CHANGELOG, `verb/board.rs`)

#### Work Order

**Goal:** `cargo-berth board --reservation <id> --json` includes that reservation's ordering edges and overlap answers.

**Spec:**
1. `ReservationReportSnapshot` (`src/output.rs` ~1058) gains `ordering_edges` and `overlap_answers`. `ordering_edges` holds every declared ordering constraint naming the reservation as predecessor or successor, waiting and settled, each in the same JSON shape the full board uses (`WaitingHold::OrderingEdge` fields, `SettledOrderingConstraint`); `overlap_answers` holds every live `RecordedOverlapAnswer` naming it. Empty arrays when none.
2. Compute both from the same report the lifecycle read already reconciles (`execute_reservation_lifecycle` → `reservation_lifecycle_snapshot`), through the helpers `BoardModel::build` uses (`declared_ordering_constraints`, `board_overlap_answers`), filtered to the reservation — one derivation for both board answers, not a second copy. When extracting the shared answer projection, replace two `Option`s with semantic outcome enums: `board/answers.rs::reservation_answers` (`src/board/answers.rs:444`), whose `Option<(ReservationId, &[ConflictAuthorization])>` conflates an operation that records no answer with an answer that has no reservation binding, gets one variant for each; and `DeferralConsequence::from_lifecycles` (`src/board/rows.rs:462`), whose `None` means both endpoints have released, returns an enum with a variant for that case. Every wire layout stays unchanged. Each `RecordedOverlapAnswer::Sequence` already carries `edge_id` (phase 4), the same `EdgeId` its `WaitingHold::OrderingEdge` or `SettledOrderingConstraint` entry carries, so an answer links to its edge through that field and the new fields add no second edge reference. A claim that widened behind an existing edge records a second `Sequence` answer with the same `edge_id`; the per-reservation filter keeps every answer and never deduplicates by edge.
3. The JSON envelope's rendered `presentation` (`reservation_lifecycle_presentation`) lists the edges and answers after the lifecycle details. The selector still requires `--json` (`worktree-coordination.md:443`).
4. Regenerate the output contract; follow the versioning rule in `docs/cargo-berth/as-built/json-contract.md` for an added payload field.
5. Tests in `tests/board.rs`: (a) a successor with a declared `--after` edge reports that edge from both the predecessor's and successor's `board --reservation` answers, for a waiting edge and a settled one; (b) an edge or answer between two other reservations is absent; (c) a reservation with a recorded overlap answer reports it, whether it asked or was answered about, and a released answer from history is not reported as live; (d) a reservation with neither reports empty arrays; (e) each entry equals the full board's entry for the same edge or answer, including an answer bound to an existing edge by a claim that widened behind it (phase 4), compared between the full board and both endpoints' `board --reservation` answers — extend `assert_existing_ordering_widen_answer` (`tests/board.rs` ~1566) for that case, where both `Sequence` answers appear with the one `edge_id`; every `Sequence` answer whose consequence is not `replaced` names an entry of the same answer's `ordering_edges` by `edge_id`; (f) extend `reservation_report_and_presentation`'s exact presentation assertion (~2183); (g) unit tests in `src/board/answers.rs` and `src/board/rows.rs`: an answer with no reservation binding, and an answer whose endpoints have both released, never become live reservation answers. Extend `reservation_report`'s helper rather than adding a second one.
6. Docs: json-contract.md "Named reservation lifecycle" (~560–620) and worktree-coordination.md (~443) describe the new fields; CHANGELOG `Fixed` line.

**Files:**
- `crates/cargo-berth/src/output.rs` — snapshot fields
- `crates/cargo-berth/src/board/report.rs` — snapshot derivation and text presentation
- `crates/cargo-berth/src/board/rows.rs`, `crates/cargo-berth/src/board/answers.rs` — shared per-reservation filtering, the semantic outcome enums and their unit tests (Spec 5(g))
- `crates/cargo-berth/src/verb/board.rs` — lifecycle path
- `crates/cargo-berth/src/output_contract.rs`, `docs/cargo-berth/generated/output-contract.json`
- `crates/cargo-berth/tests/board.rs` — the Spec 5 (a)–(f) tests
- `crates/cargo-berth/CHANGELOG.md`
- `docs/cargo-berth/as-built/json-contract.md`, `docs/cargo-berth/as-built/worktree-coordination.md`

**Workers:** 1 writer + 1 tester
- `impl` — `src/` (the Spec 5(g) unit tests included), generated contract, CHANGELOG, as-built docs; runs lint and the full gate.
- `test` — the Spec 5 (a)–(f) tests in `tests/board.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; `board --reservation <id> --json` on the live ledger shows `ordering_edges` and `overlap_answers`.

### Phase 7 — Every cargo-berth test finishes in under a second  · status: todo

**Depends on:** Phase 6 (every phase edits `crates/cargo-berth/tests/`)

#### Work Order

**Goal:** No cargo-berth test takes a second or more on a quiet machine, except where product behavior sets the floor, and each such exception is named with its cause.

**Spec:**
1. The phase 1 full gate listed 155 tests over one second (the slowest is `gate::observe_enforce_and_one_use_force_apply_to_both_deferred_endpoints` at 7.7 s), recorded at `/tmp/claude/unit/a433b49e-d462-472a-9f5b-3e767f5be8b6/slow_tests_phase1.txt`. Before changing anything, measure every cargo-berth test on a quiet machine and take that list as the target set, slowest first. **Timing rule** (the baseline and the gate both use it): one `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` run, its nextest settings unchanged, while no other build or test runs on natedev; read each test's duration from the `PASS [ N.NNNs ]` lines of that run, and a test at 1.000 s or more fails the gate. The run starts with a cold template cache: `RepositoryTemplate` (`crates/cargo-berth-test-support/src/repository_template.rs`) persists each built repository on disk across nextest processes, under `CARGO_TARGET_TMPDIR` in `cargo-berth-templates/` (`target/tmp/cargo-berth-templates/` in this worktree), keyed by the test binary and the `cargo-berth` executable it runs, so only the first test of a build pays for it. Delete that directory before the baseline run and before the acceptance run, so each test that builds a template pays the build and the `build.lock` wait as it would after a rebuild, and a rerun on an unchanged tree cannot pass the gate by reusing templates.
2. For each slow test, find where its time goes (repeated `init`, repository setup, git process count, journal replay, waits) and cut it at the cause. Shared costs go into shared fixtures in `crates/cargo-berth-test-support` or `tests/support/`, never into a weaker assertion. A test keeps the behavior it names. The target set includes the phase 2 tests `markerless_post_commit_reports_every_incursion_without_ambiguous_widens` (`tests/drift.rs:2969`; 2.6 s under full-suite load, 0.2 s alone) and the `tests/hooks.rs` resolve tests (`post_tool_use_stays_silent_after_resolve_every_committed_incursion`, `committing_an_answered_working_tree_incursion_keeps_the_hook_silent`, `a_second_commit_to_an_answered_working_tree_incursion_reopens_it`; 0.5–2.2 s under load).
3. A cut keeps every behavior distinction in the shared constraints from prior phases: upstream reachability filters authorship only, pre-phase-2 incidents keep holder-subset coverage, and a working-tree answer covers one entering commit while a further commit reopens the incident. Each fixture keeps the upstream lookup count fixed per integration target: `POST_COMMIT_ENGINE_GIT_PROCESS_CEILING` (`tests/gate.rs:109`) stays 17, one upstream lookup per distinct target.
4. A test whose floor is set by product behavior (a real timeout or a deadline the product enforces) stays. Name it in the phase summary with its measured duration and the cause.
5. Tests: re-measure the whole package under the timing rule, template cache cleared first. Every test except the named exceptions finishes in under a second; the summary lists each test's before and after duration.
6. Docs: a CHANGELOG `[Unreleased]` line only if a public test-support API changed.

**Files:**
- `crates/cargo-berth/tests/` — slow tests and their fixtures
- `crates/cargo-berth-test-support/src/` — shared fixture costs
- `crates/cargo-berth/src/` — only a test-configurable value a slow test needs, for example a deadline it can shorten
- `crates/cargo-berth/CHANGELOG.md`

**Workers:** 2 writers — ownership is by whole file, so no test file has two writers; move a file between slots after the fresh baseline only to balance the measured time.
- `impl` — `src/`, `crates/cargo-berth-test-support/`, `tests/support/`, `tests/gate.rs`, `tests/board.rs`, `tests/edges.rs`, `tests/ledger.rs`, `tests/compaction.rs`, every test file not listed for `test`, and CHANGELOG; runs lint and the full gate.
- `test` — `tests/lifecycle.rs`, `tests/drift.rs`, `tests/liveness.rs`, `tests/answers.rs`, `tests/hooks.rs`, `tests/overlap.rs`; asks `impl` for any shared fixture change.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green on a quiet machine, run after deleting `target/tmp/cargo-berth-templates/`, with no test at one second or more except the named exceptions.
