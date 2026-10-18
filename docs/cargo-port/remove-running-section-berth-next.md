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
  - `crates/cargo-berth/src/board/tui.rs` — `run(source: &mut impl BoardModelSource, model: &BoardModel)` (114). `OMITTED_MODEL_FIELDS` (72) names the five diagnostics no pane shows (`journal_position`, `recovered_bypasses_this_invocation`, `integration_order`, `targets`, `git_cost`); `board --json` still carries them. `BoardPaneDocuments::from_model` (262) drops them and each section's nested `journal_position`; the Overview document is only `overview_reservation_table` (350). `BoardModelSource` (548: `current_position`, `rebuild`), `BoardApplication::new` (576) and `refresh` (609). `event_loop` (769) polls input through `BoardEvents` (745) up to a one-second `REFRESH_INTERVAL` deadline (71); a refresh rebuilds only when the journal position moved, and a failed rebuild keeps the last good documents. Unit tests `mod tests` (1245, `TestBackend`).
  - `crates/cargo-berth/src/verb/board.rs` — `ReconciledBoardSource` (29) implements `BoardModelSource` (reconcile + `BoardModel::build`); `execute` (85); `execute_complete_board` (123: `BoardModel::build` at 128, `tui::run(source, model)` at 160); `BoardOutputSelection::ReservationLifecycleFor` (67) → `execute_reservation_lifecycle` (191), the `board --reservation <id> --json` path, unchanged by phase 5.
  - `crates/cargo-berth/src/ledger/projection.rs`, `crates/cargo-berth/src/ledger/handle.rs` — `JournalPosition::{Published, Undecodable, Unpublished}` (projection.rs:26), a movement stamp only, from `Ledger::current_journal_position` (handle.rs:363).
  - `crates/cargo-berth/src/board/rows.rs` — `BoardModel` (86–105), `BoardModel::journal_position` (515), `BoardModel::build` (606; edges via `declared_ordering_constraints(&report.constraints, &active_ids)` and `unresolved_overlaps(&report.constraints)` 639–640), `WaitingEntry` / `WaitingHold` (357, 366), `SettledOrderingConstraint` (417), `BoardJournalPosition` (151).
  - `crates/cargo-berth/src/board/answers.rs` — `RecordedOverlapAnswer` (44), `board_overlap_answers` (235).
  - `crates/cargo-berth/src/board/report.rs` — `reservation_lifecycle_snapshot(report, id)` (79), `ReservationReport` (17, 50), `reservation_lifecycle_presentation`.
  - `crates/cargo-berth/src/output.rs` — `OutputFacts::Reservation(ReservationLifecycleQueryPayload)` (~762, payload kind `"reservation"`), `ReservationLifecycleQueryPayload` (~1049), `ReservationReportSnapshot` (~1058: `reservation_id`, `lifecycle`, `race_extent`, `merge_extent`), `OutputEnvelope::reservation_lifecycle` (~1748).
  - `crates/cargo-berth/src/output_contract.rs` — schemas `reservation_lifecycle` / `reservation_lifecycle_query` (~117–124); regenerate `docs/cargo-berth/generated/output-contract.json` with `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1` (test at ~312).
  - `crates/cargo-berth/tests/ledger.rs` — init tests; `clean_init_ignores_its_untracked_configuration_and_keeps_the_legacy_line` asserts a clean `git status` after `init`; `CONFIGURATION_PATH`, `EXCLUDE_PATH`.
  - `crates/cargo-berth/tests/board.rs` — `reservation_report` / `reservation_report_and_presentation` helpers (2134, 2139), `waiting_successor_lifecycle_and_extents_are_queryable_and_listed_with_its_hold` (614).
  - `crates/cargo-berth/README.md` — "First use" (10, init at 28–57), "Board" (388–438).
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
  - Phase 5: `verb/board.rs` also owns `ReconciledBoardSource`, and the terminal board refreshes from the `JournalPosition` movement stamp. Nothing ahead may make a refresh reconcile on an unchanged journal, or bring the five `OMITTED_MODEL_FIELDS` back into a pane.
  - Phase 6: `board --reservation <id> --json` carries `ordering_edges` and `overlap_answers` from `BoardCoordinationProjection::for_reservation` (`board/rows.rs`), the projection `BoardModel::build` also uses. The `board/tui.rs` diagnostic tests check the test-local `EXPECTED_OMITTED_FIELDS`, never production `OMITTED_MODEL_FIELDS`; `tests/board.rs` keeps one `reservation_report` helper. `replaced_deferral_ordering_remains_readable_on_full_and_reservation_boards` (`tests/board.rs:1544`) guards a resolved deferral whose edge a later `--replace` retired, through real `claim --defer`, `sequence` and `claim --replace` commands; a faster fixture keeps that command sequence.

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

### Phase 5 — The terminal board shows reservations created after it opens, and its Overview holds no raw diagnostics  · status: done

**Depends on:** Phase 4 (shared README and CHANGELOG)

#### As-built

- `event_loop` (`board/tui.rs`) waits on `BoardEvents::next_event(timeout)` up to a one-second `REFRESH_INTERVAL` deadline read from `BoardEvents::now`, so sustained key or resize input never starves the refresh. `TerminalEvents` is the crossterm implementation; tests drive the loop with scripted events and `TestBackend`, and no test sleeps.
- `tui::run(source: &mut impl BoardModelSource, model)` takes the initial model and a source. `BoardModelSource` has `current_position() -> Result<JournalPosition, BoardRefreshFailure>` and `rebuild() -> Result<BoardModel, BoardRefreshFailure>`. `ReconciledBoardSource` (`verb/board.rs`) opens its own `Ledger` after the initial build and rebuilds through `reconcile::reconcile` + `BoardModel::build`.
- `Ledger::current_journal_position` (`ledger/handle.rs`) reads one `fs::metadata` on the journal plus the projection file and returns `JournalPosition::{Published, Unpublished, Undecodable}` (`ledger/projection.rs`), a movement stamp never shown or serialized. `BoardModel::journal_position` (`board/rows.rs`) gives the position a model was built at. A check rebuilds only when the position moved. `Undecodable` counts as moved, so reconcile republishes a damaged or unsupported-schema projection instead of the board stalling.
- A failed check or rebuild keeps the last good documents and position, shows "Refresh failed: <reason>" in the footer until the next good rebuild, and retries each second; it never exits the board. The selected pane and each pane's scroll offset survive a rebuild, clamped to the new document length.
- The Overview shows only the Reservations table. `OMITTED_MODEL_FIELDS` (`tui.rs`) drops `journal_position`, `recovered_bypasses_this_invocation`, `integration_order`, `targets` and `git_cost`, plus each `BoardSection`'s nested `journal_position`, from every pane; every other model field has exactly one pane. `board --json` is unchanged and still carries these fields.
- README "Board", CHANGELOG `Fixed`, `parallel-phases.md` and `worktree-coordination.md` state the live refresh and that the diagnostics are in `board --json`.

**Files:**
- `crates/cargo-berth/src/board/tui.rs` — `event_loop`, `BoardEvents`, `BoardModelSource`, `BoardRefreshFailure`, `OMITTED_MODEL_FIELDS`, pane documents and the refresh unit tests
- `crates/cargo-berth/src/verb/board.rs` — `ReconciledBoardSource`, `execute_complete_board`; `execute_reservation_lifecycle` unchanged
- `crates/cargo-berth/src/ledger/handle.rs`, `src/ledger/projection.rs`, `src/ledger/mod.rs` — `current_journal_position`, `JournalPosition`
- `crates/cargo-berth/src/board/rows.rs` — `BoardModel::journal_position`
- `crates/cargo-berth/README.md`, `crates/cargo-berth/CHANGELOG.md`, `docs/cargo-berth/as-built/parallel-phases.md`, `worktree-coordination.md`

**Binds later work:** `verb/board.rs` holds `ReconciledBoardSource` beside `execute_complete_board`; `board --reservation <id> --json` never reaches the terminal board. The diagnostic tests `pane_documents_omit_raw_diagnostics_and_nested_positions` and `every_model_fact_is_reachable_through_exact_pane_documents` check against production `OMITTED_MODEL_FIELDS`; "A single-reservation board answer carries its ordering edges and overlap answers" gives them a test-local `EXPECTED_OMITTED_FIELDS` oracle.

**Gotchas:**
- A refresh on an unchanged journal must not reconcile.
- `event_loop` is generic over `Backend<Error: Error + Send + Sync + 'static>` so `TestBackend` drives it.
- While the diagnostic tests read `OMITTED_MODEL_FIELDS`, they cannot catch a field moved into a pane and dropped from that constant.

**Ruled out:** a new pane for the diagnostics (showrunner, 2026-10-16: fewer things on screen).

### Phase 6 — A single-reservation board answer carries its ordering edges and overlap answers  · status: done

**Depends on:** Phase 5 (shared README and CHANGELOG, `verb/board.rs`)

#### As-built

- `board --reservation <id> --json` (`ReservationReportSnapshot`, `output.rs`) carries `ordering_edges: Vec<ReservationOrderingEdge>` and `overlap_answers: Vec<RecordedOverlapAnswer>`, empty arrays when none. `ReservationOrderingEdge::{Waiting { hold, edge }, Settled(SettledOrderingConstraint)}` (`board/rows.rs`) serializes in the full board's shapes, a waiting entry without its `reservation` row. `overlap_answers` holds the live answers naming the reservation as requester or holder; a released answer from history is not live.
- `BoardCoordinationProjection` (`board/rows.rs`) is the one derivation for both answers: `BoardModel::build` places its edges and answers on the full board, and `for_reservation(reservation_id) -> ReservationCoordination` keeps the edges with the reservation at either endpoint and its live answers, so each entry equals the full board's entry. The filter never deduplicates by `edge_id`: a widen behind an existing edge keeps both `Sequence` answers.
- `ReservationAnswers::{NoAnswer, Unbound, Bound { reservation_id, authorizations }}` (`board/answers.rs`) replaces `Option<(ReservationId, &[ConflictAuthorization])>`; `DeferralIntegrationOutcome::{Held(DeferralConsequence), BothReleased}` (`board/rows.rs`) replaces the `None` of `DeferralConsequence::from_lifecycles`. `DeclaredOrderingConstraints.holds` is `Vec<WaitingOrderingConstraint>`; conversion to `WaitingHold::OrderingEdge` happens only at full-board placement.
- `ResolveDefer` answers derive their consequence through `sequence_consequence`, so a deferral resolved by `sequence` and later retired by `claim --replace` reads `replaced` with the retiring reservation instead of failing the full board and every named report as `ledger_unreadable`.
- The rendered `presentation` lists `Ordering edges` then `Overlap answers` after the lifecycle and extent details. Other wire layouts are unchanged; the regenerated output contract stays version 6.
- `board/tui.rs` tests check the test-local `EXPECTED_OMITTED_FIELDS` (the five diagnostic names); production `OMITTED_MODEL_FIELDS` is unchanged.
- json-contract.md "Named reservation lifecycle", worktree-coordination.md and CHANGELOG `Fixed` describe the new fields.

**Files:**
- `crates/cargo-berth/src/output.rs` — snapshot fields
- `crates/cargo-berth/src/board/rows.rs` — `BoardCoordinationProjection`, `ReservationOrderingEdge`, `DeferralIntegrationOutcome`, `DeclaredOrderingConstraints`; unit test `both_released_deferral_endpoints_leave_no_live_reservation_answers`
- `crates/cargo-berth/src/board/answers.rs` — `ReservationAnswers`, `ResolveDefer` through `sequence_consequence`; unit test `unbound_answers_never_become_live_reservation_answers`
- `crates/cargo-berth/src/board/report.rs`, `board/overview.rs`, `board/mod.rs`, `verb/board.rs` — snapshot derivation and presentation
- `crates/cargo-berth/src/board/tui.rs` — `EXPECTED_OMITTED_FIELDS` in `mod tests`
- `crates/cargo-berth/tests/board.rs` — per-reservation edge and answer tests, the extended `assert_existing_ordering_widen_answer` and `reservation_report_and_presentation`, regression `replaced_deferral_ordering_remains_readable_on_full_and_reservation_boards`
- `docs/cargo-berth/generated/output-contract.json`, `crates/cargo-berth/CHANGELOG.md`, `docs/cargo-berth/as-built/json-contract.md`, `docs/cargo-berth/as-built/worktree-coordination.md`

**Binds later work:** "Every cargo-berth test finishes in under a second" keeps the test-local `EXPECTED_OMITTED_FIELDS` oracle, the single `reservation_report` helper in `tests/board.rs`, and the real `claim --defer`, `sequence` and `claim --replace` commands in `replaced_deferral_ordering_remains_readable_on_full_and_reservation_boards`.

**Gotchas:**
- A board answer derivation must never require a retired edge to exist: `--replace` removes edges from the live graph while their answers stay in the journal.
- Several `tests/board.rs` tests cross a second under machine load and run near 0.2 s otherwise.

**Ruled out:** a CHANGELOG `Fixed` line for the retired-deferral repair — the defect and `--replace` are both inside `[Unreleased]`.

### Phase 7 — Every cargo-berth test finishes in under a second  · status: done

**Depends on:** Phase 6 (every phase edits `crates/cargo-berth/tests/`)

#### As-built

- Test fixtures build each git repository once per target directory and copy it per test. `RepositoryTemplate` persists built repositories under `CARGO_TARGET_TMPDIR/cargo-berth-templates/`, one per test binary and `cargo-berth` executable, so only the first test of a build pays for the build.
- `IntegrationRepository::build_main_template` and `IntegrationRepository::from_template` give integration tests a prebuilt main checkout; `tests/support/reanchoring.rs` holds the `REWRITE_MAIN` template for rewrite fixtures.
- The answers, hooks, overlap, lifecycle, gate, edges, engine_instructions and board tests instantiate templates instead of running `git init` and `berth init` per test. Branch switches inside fixtures skip hooks. Git is found by a PATH scan that requires the executable bit.
- Cold package run: 19.5 s against a 52.3 s baseline, 1128/1128 passing.

**Files:**
- `crates/cargo-berth-test-support/src/repository_template.rs` — template persistence and instantiation
- `crates/cargo-berth-test-support/src/integration_repository.rs` — `build_main_template`, `from_template`
- `crates/cargo-berth/tests/support/reanchoring.rs` — the `REWRITE_MAIN` template

**Gotchas:**
- Deleting `target/tmp/cargo-berth-templates/` forces a cold build of every template; a timing run starts there, or reused templates hide fixture cost.
- Per-test times on a loaded machine swing several-fold (0.2 s tests read 1.5 s). The per-test under-a-second bar is read from the CI Test Suite nextest log, because a loaded machine cannot judge it.

**Ruled out:**
- Spawning a shell to locate git: it costs real time per lookup; the PATH scan replaces it.

### Phase 8 — The last 18 cargo-berth tests under a second  · status: done

**Depends on:** Phase 7 (every phase edits `crates/cargo-berth/tests/`)

#### As-built

- The 25 slowest cargo-berth integration targets from the CI Test Suite log at d1ced22c do less git work, each cut at its cause; every behavior assertion stays. Alone on natedev with warm templates they run 0.05–0.27 s.
- `tests/edges.rs` successor setup no longer re-reconciles the growing ledger. `tests/board.rs` reuses the historical proof fixture. `tests/hooks.rs` runs the peer on the main checkout.
- `tests/support/reanchoring.rs` builds rewrite fixtures with `commit --only`, reads loose refs directly, and keeps the holder checkout inside the `REWRITE_MAIN` template. `tests/liveness.rs` instantiates the `RECOVERY_SOURCES` template and builds both recovery dispositions from one hooked commit.
- `tests/gate.rs` `trunk_gate_remembers_first_target_landing_after_target_revalidation` records its ordering edge with one `claim --after` instead of a deferral plus `sequence`. `POST_COMMIT_ENGINE_GIT_PROCESS_CEILING` stays 17, and the cold-cost bounds of `successor_board_has_fixed_cold_cost` and `cold_proof_subjects_bound_git_evaluation_for_distinct_and_duplicate_reservations` are unchanged.
- `tests/lifecycle.rs` checks the admitted outsider before reading the session mapping. `tests/answers.rs` `an_answer_to_a_holder_with_many_scopes_records` protects 300 scopes with real work through one claim widened twice, asserts the journalled protection holds all 300, and asserts the answer binds the refusal's `overlap_scope_revision`.
- No product code, `cargo-berth-test-support` API or CHANGELOG line changed.

**Files:**
- `crates/cargo-berth/tests/support/git_configuration.rs` — `append_git_configuration`, writes fixture git config without spawning git
- `crates/cargo-berth/tests/support/linked_checkout.rs` — `administrative_directory`, resolves a linked checkout's gitdir pointer
- `crates/cargo-berth/tests/support/loose_refs.rs` — `read_loose_ref`, reads a loose ref without spawning git
- `crates/cargo-berth/tests/support/lock_contention.rs` — `LOCK_CONTENTION_TOLERANCE`, moved from `timing.rs`
- `crates/cargo-berth/tests/support/reanchoring.rs` — the `REWRITE_MAIN` template with the holder checkout
- `crates/cargo-berth/tests/{answers,board,drift,edges,gate,hooks,lifecycle,liveness}.rs` — the cut targets

**Gotchas:**
- Time each target alone, one `verify.sh test cargo-berth --filter <target>` run per target. A run naming many targets measures contention among them, not each test's cost.
- natedev load (17 to 230 during this work) voids per-test times; the CI Test Suite nextest log on main after merge is the binding gate for the under-a-second bar, and a target still at 1 s or more there needs a fixture redesign in a new phase.

**Ruled out:**
- A speedup that drops a proof: the clean-claim and many-scope answer tests keep proving the behavior they name.

### Phase 9 — cargo-port's Git pane shows the Error row for a worktree whose gitdir target is missing  · status: done

**Depends on:** none (cargo-port only; runs after phase 8 by the showrunner's call)

#### As-built

- Selecting a linked worktree whose `.git` file names a missing gitdir shows the Git pane's red `Error` row, `broken .git — gitdir target missing`, first and no Branch row, also after the live checkout probe arrives. The pane title names no branch for it.
- `GitCheckout::{Broken, AwaitingProbe, Readable { head: HeadState, relation: Option<HeadRelation> }}` replaces the `head` / `head_relation` pair on `GitData` and `GitDetailFields`. `build_git_detail_fields` builds it with `GitCheckout::from_probe(WorktreeHealth, Option<HeadState>, Option<HeadRelation>)` from the selected entry's own `WorktreeHealth` (`project_list.at_path(..)`, default `Normal`); a broken worktree cannot carry a HEAD by construction. `AwaitingProbe` is a pending probe or a path with no checkout info.
- `git_fields_from_data` emits `DetailField::WorktreeError` first when `Broken` and `Head` only when `Readable`. `git_panel_title`, `copy_data` and Git pane availability read the same `GitCheckout`; `PaneId::Git` is available whenever `checkout` is not `AwaitingProbe`, so a broken worktree with no remotes or worktrees still reaches the Git pane.
- `tui_pane::SemanticTheme::on_error` (serde default white) with the `on_error_color()` accessor, re-exported from `tui_pane`; every built-in theme in tui_pane, cargo-port, cargo-tile and cargo-handler sets it white. The Error row draws `fg(on_error_color()).bg(error_color())`, about 7.7:1 in the light theme.

**Files:**
- `crates/cargo-port/src/tui/panes/pane_data/git_data.rs` — `GitCheckout`, `from_probe`, `git_fields_from_data`, test `broken_worktree_error_precedes_head_and_normal_worktree_omits_it`
- `crates/cargo-port/src/tui/panes/pane_data/{detail_data,detail_field,copy_data,mod}.rs`, `crates/cargo-port/src/tui/panes/mod.rs` — carry and re-export `GitCheckout`
- `crates/cargo-port/src/tui/panes/git/render.rs` — Error row style; `git_panel_title` over `GitCheckout`
- `crates/cargo-port/src/tui/app/mod.rs` — Git pane availability; test `selecting_worktree_with_missing_gitdir_renders_git_error` (real missing-gitdir directory, live probe, rendered text and Error cell colors in dark and light)
- `crates/tui_pane/src/theme/{mod,accessors,fallback,loader}.rs`, `crates/tui_pane/src/lib.rs` — `on_error` token, accessor, defaults, test `existing_theme_files_default_on_error_and_allow_an_override`
- `crates/cargo-port/src/themes/builtins.rs`, `crates/cargo-tile/src/theme/builtins.rs`, `crates/cargo-handler/src/theme.rs` — `on_error` in each `SemanticTheme` literal

**Binds later work:** `SemanticTheme::on_error` and `tui_pane::on_error_color()` are the foreground for text on the error red; the phase "cargo-port's broken-worktree rows in the project list draw white text on the error red" applies them to the remaining error-red text sites and adds `on_error` to the shipped theme templates.

**Gotchas:**
- `make_app`'s quiet startup skips the git probe; a test of Git rows for a real checkout applies the probe result (`CheckoutInfo::get` → `BackgroundMsg::CheckoutInfo`), or it reads unfetched metadata.
- `CheckoutInfo::get` accepts a gitfile whose gitdir is missing, and `resolve_head_state` (`project/git/checkout.rs:272`) reports any unresolvable HEAD as `HeadState::Unborn`; `GitCheckout::from_probe` drops it when health is `Broken`. Other readers of that `Unborn` (finder label, CI and project-list `branch_name()`) treat it as no branch.
- A new `SemanticTheme` field is set in every theme literal across tui_pane, cargo-port, cargo-tile and cargo-handler and needs a serde default so existing theme files load; run all four suites.

**Ruled out:**
- Default text on the error red: black on the light theme's error red reads 2.71:1.

### Phase 10 — Every refused check or claim leaves one typed journal record naming its requester, paths and holders  · status: done

#### As-built

- `JournalOperation::OverlapRefused { verb: RefusedVerb, scopes: ReservationScopeSet, holders: RefusingHolderSet }` follows `Answer` in `declare_journal_operations!`. `RefusedVerb::{Check, Claim}` is snake_case on the wire and separate from `output::CommandVerb`, so no other verb can be recorded. `RefusingHolder { reservation_id, scopes }` pairs each holder with its own `overlapping_scopes`, so a union of several holders' scopes is unrepresentable. `RefusingHolderSet` / `EmptyRefusingHolderSet` come from `nonempty_journal_set!`'s journal-only branch (no `JsonSchema`, no accessor), built by `TryFrom<&[ReservationConflict]>` in conflict order.
- The requester is the record's own `actor` plus `identity_inputs`, never a copy inside the operation: for `claim` the claim's `journal_mutation_actor`, for `check` `journal_mutation_actor_for(coordination_run_id)` from `decide` (an unidentified caller gets a fresh run id per process).
- `verb::refusal::record(worktree_context, requester, verb, scopes, conflicts)` opens the ledger and appends in its own record-only `Ledger::transact` with an `Infallible` validator, after the verb settles on its refusal. It returns nothing: on lock contention, an unreadable ledger or an empty holder set it writes nothing and the refusal envelope is unchanged. `TransactionValidation::Reject` stays append-free.
- Exactly one record per `blocked_by_overlap` envelope from `check` or `claim`, none for any other status. `check`: `CheckDecision` carries `requester` and `worktree_context`; it records at `reconcile_and_retry`'s final snapshot refusal and at `render_acquisition`'s `Blocked` arm (`PostWriteIncursion` is a separate arm that records nothing). `claim`: `acquire_new` and `named::acquire` record through `audited_claim_rejection_execution` / `record_refusal` after the refusing transaction returns; only `Blocked` and `IncompleteAnswer` record, and every other `ClaimExecution` variant is named.
- Readers: no-op arms in `RetainedReservationSet::apply` and `OrderingGraph::replay`; `record_ceiling` is `Declared`; `is_coordination_record` is false, so no board, alert, edge or enrollment reader sees it; compaction keeps it forever. `CURRENT_SCHEMA_VERSION` stays 2; `reader_compat` fixtures and `output-contract.json` are byte-identical; no envelope changes.
- `tests/support/overlap_refusal.rs` `assert_one_refusal_appended(journal_before, journal_after, verb, envelope)` checks one appended line whose holders and scopes match `blocked_by` and the envelope conflicts. `cargo-berth-test-support` holds `board_reservation_entries` / `board_reservation_ids`, the one list of board sections.

**Files:**
- `crates/cargo-berth/src/ledger/journal.rs` — the operation, value types, macro branch, ceiling and classification, unit tests
- `crates/cargo-berth/src/verb/refusal.rs` — the best-effort recorder
- `crates/cargo-berth/src/verb/check.rs` — two recording sites
- `crates/cargo-berth/src/verb/claim/mod.rs` — `PreparedClaim::transact`, `audited_claim_rejection_execution`, `record_refusal`
- `crates/cargo-berth/src/verb/claim/named.rs` — named-phase claim recording
- `crates/cargo-berth-test-support/src/integration_repository.rs` — board section walk
- `crates/cargo-berth/tests/support/overlap_refusal.rs`, `tests/overlap.rs`, `tests/answers.rs`, `tests/lifecycle.rs` — refusal-record assertions
- `docs/cargo-berth/as-built/json-contract.md`, `docs/cargo-berth/as-built/worktree-coordination.md` (27 variants), `crates/cargo-berth/README.md`, `crates/cargo-berth/CHANGELOG.md`

**Binds later work:** `acquire_new` reads `ClaimRequest` field by field rather than by exhaustive destructuring, and `claim_execution_from_outcome`'s `LedgerTransactionOutcome::Rejected` arm is unreachable. Any board-section walk uses `board_reservation_entries` / `board_reservation_ids`. `tests/lifecycle.rs` includes the `overlap_refusal` module and asserts one refusal record in `fast_path_declines_a_retained_foreign_conflict_on_its_held_path`.

**Gotchas:** An older cargo-berth decodes an `overlap_refused` line as a corrupt interior record and finds the ledger unreadable; every machine sharing a repository runs this version. Each refusal moves `JournalPosition`, so an open terminal board rebuilds once. Each refused hook check adds about 0.5 KB to the journal for good. Two phases verifying in one worktree change the tree mid-run and leave the package pass unrecorded.

**Ruled out:** writing the record inside the rejecting transaction — the common `check` refusal holds no lock, a retried first `Blocked` attempt would record a refusal never returned, and `FirstTouchClaimRejection::Blocked` also serves post-write incursions; copying the requester into the operation — it could disagree with the record's `actor`; a retention rule for refusal records — the style pilot needs the full count.

### Phase 11 — The 16 cargo-berth tests still at 1 s in CI finish in 0.7 s  · status: done

**Depends on:** phase 10 (`crates/cargo-berth/`)

#### As-built

- Recorded templates: `LedgerTemplate { repository, ledger: LedgerFixture, build_timing: TemplateBuildTiming }` (`repository_template.rs`) wraps a `RepositoryTemplate`, so existing template literals are unchanged. `LedgerFixture::Recorded` keeps journal records and relocates paths inside `.git/cargo-berth`; after each copy the projection and replay checkpoint are deleted so the next command replays the journal. `IdentityRenewal` maps every repo, main-worktree and linked-worktree identity to a fresh `Uuid::now_v7()` across the whole snapshot (files, journal actors, markers), so copied reservations pass `validate_same_owner`. `RECORDED_TEMPLATE_LIFETIME` rebuilds a recorded template older than 12 h, below the 24 h staleness rule.
- `TemplateBuildTiming::{InTest, BeforeRun}`: a `BeforeRun` template copies from `$CARGO_BERTH_PREBUILT_TEMPLATES/<name>`; a missing build, or the variable unset, builds it in the test, never a panic.
- Before-run build: `.config/nextest.toml` has `experimental = ["setup-scripts"]` and setup script `cargo-berth-templates` = `crates/cargo-berth/tests/setup-recorded-templates.sh` relative to the workspace root, filter `package(cargo-berth)`. The wrapper runs `${CARGO_TARGET_DIR:-target}/tmp/cargo-berth-templates/builder` only when `builder-run-id` equals `NEXTEST_RUN_ID`, else exits 0. `tests/recorded_templates.rs` (`[[test]]`, `harness = false`): `--list` replaces the `builder` symlink with its own executable and writes `builder-run-id`; a plain run builds the 15 `BeforeRun` templates in parallel under `before-run/<key>/<name>`, runs each builder's setup assertions, and appends `CARGO_BERTH_PREBUILT_TEMPLATES` to `$NEXTEST_ENV`. The build is reported as its own `SETUP PASS` line (natedev cold 2.3–2.9 s).
- Templates: `edges_templates.rs` — `RECORDED_PROJECTION`, `CROSS_TARGET_PAIR`, `CROSS_TARGET_AFTER_PAIR`, `LEGACY_NEGATIVE_{UNRELATED,PROTECTED}_GAP` (via exhaustive `const fn legacy_negative_template(gap: SuccessorGap)`), `LEFT_RIGHT_TREES`; `gate_templates.rs` — `ENROLLED_PAIR`, `UNENROLLED_PAIR`, `REVALIDATED_CROSS_TARGET_LANDING`; `reanchoring_templates.rs` — `TWO_REBASE_HISTORY`, `STACKED_UPPER_CHECKPOINT`, `SPLIT_LOWER_CHECKPOINT_{ACTIVE_RETENTION,UPDATED_SIBLING}`, `DRAINED_SPLIT_{RETENTION,SIBLING}_CHECKPOINT`. `IntegrationRepository` gains `CrossTargetPair`, `CrossTargetOrdering`, `build_cross_target_pair`, `cross_target_pair`, `recorded_lane` and `git_without_hooks`.
- Test splits: `legacy_negative_…` runs once per `SuccessorGap`; liveness splits into `abandon_and_retire_dispositions_remain_distinct_after_replay` and `rewritten_integration_replacement_validates_evidence`; the split drop tails are `dropping_a_split_hunk_keeps_the_{retention,updated_sibling}_checkpoint_outstanding` on the drained templates; the enrollment variants run only gate mode and `integrate` on `ENROLLED_PAIR`. Every rebase and `drain_markers` stays live and hooked; landing moves that only feed board or `sequence` assertions run hookless. `two_rebase_history_copies_keep_owners_and_drain_their_markers` proves two copies hold different identities and each drains its markers.
- `a_derived_record_past_its_own_ceiling_is_not_reported_as_caller_input` sizes its record from `MAXIMUM_DERIVED_JOURNAL_RECORD_BYTES` and `MAXIMUM_JOURNAL_RECORD_BYTES` (65 long paths today).
- Claims: every `ClaimRequest` read destructures it exhaustively (`claim_overlap_authorization` in `cli.rs`); `ClaimAnswerPolicy<CallerRequest>` (`CallerRequest`, `RepositoryDefault`) replaces `ClaimDefaultAnswer` and the paired authorization/default fields from validation through `OutputEnvelope::answered`; `claim_execution_from_outcome` has no `Rejected` arm. Output text and JSON are unchanged.
- Natedev cold full run: 1140/1140; the upper-checkpoint pair reads 0.60–0.64 s; `split_checkpoint_survives_an_active_phases_rewritten_retention_ref` 2.102 s, `split_checkpoint_survives_stacked_branches_rewritten_with_update_refs` 2.587 s and `split_checkpoint_survives_switching_the_issuing_checkout_before_board` 1.849 s stay over target.

**Files:**
- `crates/cargo-berth-test-support/src/repository_template.rs` — `LedgerTemplate`, `LedgerFixture`, `TemplateBuildTiming`, `IdentityRenewal`, before-run copy
- `crates/cargo-berth-test-support/src/integration_repository.rs`, `lib.rs` — cross-target pair helpers, re-exports
- `.config/nextest.toml`, `crates/cargo-berth/Cargo.toml` — setup script and builder target
- `crates/cargo-berth/tests/recorded_templates.rs`, `tests/setup-recorded-templates.sh` — the builder and its wrapper
- `crates/cargo-berth/tests/support/{edges,gate,reanchoring}_templates.rs` — template builders
- `crates/cargo-berth/tests/{edges,gate,lifecycle,liveness}.rs`, `tests/support/reanchoring.rs` — tests on templates
- `crates/cargo-berth/src/verb/claim/{mod,named}.rs`, `src/cli.rs`, `src/output.rs` — `ClaimRequest` destructuring, `ClaimAnswerPolicy`
- `crates/cargo-berth/src/ledger/handle.rs` — derived-record unit test

**Binds later work:** "The full cargo-berth suite passes on the Mac, every test under a second" owns the three split_checkpoint tests over target (2.102 s, 2.587 s, 1.849 s on natedev cold) and the cold single-target `--test <target>` run, which never lists `recorded_templates`, so the wrapper skips the builder and the target builds its templates in-test.

**Gotchas:**
- The wrapper needs nextest >= 0.9.138 (it sets `NEXTEST_RUN_ID` while listing) and the default target directory; otherwise a full run silently builds every `BeforeRun` template in-test and `SETUP PASS` reads milliseconds.
- A template's build counts only in `SETUP PASS`, never in a test's time; test times are read from a full parallel package run, never a filtered or single-test run.

**Ruled out:** subtracting the before-run build from any test's time; a panic on a missing before-run build.

### Phase 12 — cargo-port's broken-worktree rows in the project list draw white text on the error red  · status: done

#### As-built

- A `WorktreeHealth::Broken` row's name-column style in `row_to_line` and the ` [broken]` Disk suffix style in `disk_suffix_for_state` are `fg(on_error_color()).bg(error_color())` — white on the error red in every built-in theme, matching the Git pane's Error row.
- Every `[semantic]` table of every shipped theme template sets `on_error = "White"`: cargo-port's `default_dark`, `default_light` and `high_contrast` (two tables), and the same plus `starter.toml` in cargo-tile and cargo-handler.
- `broken_worktree_project_row_draws_on_error_text_on_error_red` renders through `render_tree_buffer` at exactly `widths.total_width()`, in Dark and Light, with a 1 GiB primary; it checks every drawn name cell and every visible suffix cell and requires at least ` [` to be drawn.
- Test guard `ActiveThemeOverride::set(appearance)` restores the saved theme in `Drop`; `assert_git_error_colors` uses it.
- `refreshed_worktree_metadata_appends_into_existing_group` runs in 0.037 s: `create_refresh_worktree` writes the linked-worktree layout (`.git` pointer + `commondir`) by hand instead of spawning git; `WorktreeProjectKind::write_manifest` replaces the free helper.

**Files:**
- `crates/cargo-port/src/tui/columns/project.rs` — broken-row style
- `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs` — ` [broken]` suffix style
- `crates/cargo-port/src/tui/app/mod.rs` — the broken-row test, `ActiveThemeOverride`, refresh-test fixture
- `crates/{cargo-port,cargo-tile,cargo-handler}/themes/*.toml` — `on_error` key
- `crates/cargo-port/CHANGELOG.md` — `[Unreleased]` `Fixed` line

**Binds later work:** Tests that set a theme restore it through `ActiveThemeOverride::set(appearance)`, never a manual save and restore. The broken-row test's comment says the suffix tail is clipped; the Disk column width fix tightens it to require the whole ` [broken]` and rewrites that comment.

**Gotchas:** The Disk column clips ` [broken]` at product bounds; with an unknown disk size (`0 B`) none of the suffix is drawn.

**Ruled out:** Widening the test buffer past `widths.total_width()` — it hides the clipping; changing product column widths here.

### Phase 13 — cargo-port reports a checkout whose HEAD git cannot read as unreadable, never as unborn  · status: done

**Depends on:** phase 9 (`GitCheckout`) and phase 15 (shared `app/mod.rs`, `CHANGELOG.md`)

#### As-built

- Probe: `HeadState` has a fourth variant, `Unreadable` (serde `"unreadable"`): git could not read `HEAD` — the HEAD file or the ref it names is missing or corrupt. `branch_name()` is `None` and `display_label()` is `""` for it (the finder column shows nothing, as for `Unborn`). `branches::get_current_branch` returns `Some` only on exit 0.
- `resolve_head_state` (private to `checkout.rs`): a branch name → `Branch(name)` (one git call on the common path); `HEAD` → `rev-parse --verify -q --short=8 HEAD`, exit 0 with non-empty stdout → `Detached { short_sha }`, else `Unreadable`; no name → `unborn_or_unreadable(repo_root) -> HeadState` runs `symbolic-ref -q HEAD`: exit 0 → `Unborn`, any other exit or spawn error → `Unreadable`.
- Pane: `GitCheckout` is one flat enum, `{ Broken, AwaitingProbe (default), UnreadableHead, Unborn, Detached { short_sha }, Branch { name, relation: HeadRelation } }`; no variant holds an `Option` or a `HeadState`, so a branch always carries its relation and no other state has one. `GitCheckout::from_probe(health, head: HeadProbe<'_>, default_branch: Option<&str>, placement: WorktreePlacement)`: `Broken` health wins over any probe; `Awaiting` → `AwaitingProbe`; `Probed` maps each `HeadState` to its variant, a branch through `HeadRelation::classify(branch: &str, default_branch: Option<&str>, placement: WorktreePlacement) -> Self`.
- `HeadProbe<'a> { Awaiting, Probed(&'a HeadState) }` and `WorktreePlacement { Linked, NotLinked }` are private boundary types in `git_data.rs`, built in `build_git_detail_fields`; `WorktreePlacement::of(Option<&WorktreeStatus>)` is the only reader of that `Option` (`Linked { .. }` → `Linked`, everything else and `None` → `NotLinked`). Every match on `GitCheckout` is exhaustive with no `_` arm.
- Display: the Branch row reads `HEAD_UNREADABLE_LABEL` (`"HEAD unreadable"`, `tui/panes/pane_data/constants.rs`, `pub(in crate::tui)`, re-exported `cfg(test)` with `HeadRelation` through `pane_data/mod.rs` and `panes/mod.rs`) in `error_color()`. `field_value_style(data: &GitData, field: DetailField, value: &str) -> Style` (`git/render.rs`) picks the style from the typed state (`matches!(data.checkout, GitCheckout::UnreadableHead)`), never the drawn string. `build_field_value` drops the ` ({GIT_LOCAL} local)` suffix only after `HEAD unreadable`. The title is `Git` and the copy value `""` for an unreadable HEAD.
- Tests find drawn text only through `find_text_cell(&Buffer, &str) -> Option<(u16, u16)>` (`app/mod.rs`); the linked-worktree fixture helper is `write_linked_worktree_layout`. Tests: `resolve_head_state_reports_unreadable_head_not_unborn` (corrupt `.git/HEAD`, corrupt branch ref, unborn, intact), `unreadable_head_shows_branch_row_as_head_unreadable`, `selecting_checkout_with_unreadable_head_renders_head_unreadable` (Dark and Light, error-color value cells).

**Files:**
- `crates/cargo-port/src/project/git/checkout.rs`, `branches.rs` — `HeadState::Unreadable`, `resolve_head_state`, `unborn_or_unreadable`, exit-0 branch read
- `crates/cargo-port/src/tui/panes/pane_data/git_data.rs` — flat `GitCheckout`, `HeadProbe`, `WorktreePlacement`, `HeadRelation::classify`, `from_probe`
- `crates/cargo-port/src/tui/panes/pane_data/{detail_field,copy_data}.rs`, `tui/panes/git/render.rs` — Branch value, copy value, title, `field_value_style`, ` (local)` suffix rule
- `crates/cargo-port/src/tui/panes/pane_data/{constants,mod}.rs`, `tui/panes/mod.rs` — `HEAD_UNREADABLE_LABEL` and the test re-exports
- `crates/cargo-port/src/tui/app/mod.rs` — pane test, `find_text_cell`, fixtures
- `crates/cargo-port/CHANGELOG.md` — `[Unreleased]` `Fixed` line

**Binds later work:** "cargo-port shows every git read that fails as unreadable, never as clean, present or absent" builds on `HeadState::Unreadable`, the flat `GitCheckout`, `unborn_or_unreadable` and the Branch row `HEAD unreadable` in `error_color()`; its `field_value_style` becomes one `match` on a typed value style (review nit F002). `resolve_head_state` is private to `checkout.rs`.

**Gotchas:**
- The detached-SHA probe and `get_current_branch` still carry `.filter(|o| o.status.success())`, because `git_output_logged` returns `Ok` for any exit status.
- A broken linked worktree now probes as `Unreadable`; `GitCheckout::Broken` still wins, so its pane shows the Error row.
- `CiState::is_unpublished_branch` answers `true` for every non-branch checkout (now including unreadable) while `ProjectList::unpublished_ci_branch_name` answers `None` for the same checkout.

### Phase 14 — cargo-port shows every git read that fails as unreadable, never as clean, present or absent  · status: done

**Depends on:** phases 15 and 13

#### As-built

- Runner (`project/git/command.rs`): every git spawn goes through `git_output_logged(repo_root, op, args: &[&str]) -> Result<GitStdout, GitReadError>` (`Ok` only on exit 0) or `git_answer_logged(..) -> Result<GitAnswer, GitReadError>` (`0 → Yes(GitStdout)`, `1 → No`, else `Err`) for commands whose exit 1 means "no" (`show-ref --verify --quiet`, `check-ignore -q`, `symbolic-ref -q`, `rev-parse -q --verify HEAD`, `config --get-regexp`). `git_command` is private; no caller reads a raw `Output`. `GitReadError { op, cause: GitReadCause }` (`pub(super)`, `Display` + `Error`) logs once at `warn`; private `GitReadCause { NotStarted(ErrorKind), Exited { code, stderr }, Signalled, Unparsable }`. `GitStdout` exposes `trimmed()` and `lines()`.
- Result types, each with an `Unreadable` variant and no `Option` meaning both "absent" and "failed": `GitStatus::Unreadable` (label `GIT_UNREADABLE_LABEL` `"unreadable"`, icon `GIT_STATUS_UNREADABLE` `"⛔"`, both in `src/constants.rs`); `CommitDate { At, NoCommits, Unreadable }`; `TrackedRef { Ref, Untracked, Unreadable }`; `AheadBehind { ahead, behind }` and `Comparison { Counted, NotCompared, Unreadable }`; `BisectProgress::Unreadable`; `ResolvedBranch { Named, NotFound, Unreadable }` and `RefPresence { Present, Absent }` (`branches.rs`); `RemoteList { Listed, Unreadable }` with `listed() -> &[RemoteInfo]`, `FirstCommit { AwaitingProbe, Probed(CommitDate) }` (`repo/info.rs`); `GitOrigin::Unreadable`; `PinnedCommit { Sha, NotPinned, Unreadable }` (`submodule.rs`).
- Probes: `CheckoutInfo::get(probe_path, local_main_branch: &ResolvedBranch)`; an unborn or unreadable HEAD sets `last_commit` and `ahead_behind_local` without spawning git, and an unborn HEAD compares as `NotCompared`. `get_upstream_branch(repo_root, &HeadState) -> TrackedRef` reads `for-each-ref --format=%(upstream:short) refs/heads/<b>` (empty → `Untracked`). `RepoInfo::get` takes its head from `resolve_head_state` (`pub(super)`); a failed `git remote`, `remote get-url` or pushurl read makes `remotes` `RemoteList::Unreadable`, while a per-remote tracked-ref or count failure stays on that remote. `get_first_commit -> CommitDate` falls back to `rev-parse -q --verify HEAD` to tell `NoCommits` from `Unreadable`. Submodule pins: `ls-tree HEAD` failure on an unborn parent → `NotPinned`, otherwise `Unreadable`.
- Failure ownership (`tui/panes/pane_data/formatting.rs`): `GitReadFailures::of(Option<&CheckoutInfo>, Option<&RepoInfo>)` maps each failed read to the row that owns it (private `UnreadableGitRow { Field(DetailField), RemoteTracked(usize), RemoteSync(usize) }`); a failed default-branch, local-main or upstream read is owned by the `Head` row. `GitReadHealth { Readable, Unreadable }` and `GitSyncLabels { origin, main }` derive from the same set, so the list marker and the pane rows agree. `ProjectList::git_row_status_for` reports `GitStatus::Unreadable` for any failed read or an unreadable submodule pin; `worst_git_status` ranks `Unreadable` above `Modified`; any failed read sets both compact sync cells (Og/Mn) to `⛔`.
- Git pane: `GitData` holds `failed_reads: GitReadFailures` and typed values (`GitProbe<T> { AwaitingProbe, Probed(T) }` for `vs_local`, `local_main_branch`, `last_commit`; `inception: FirstCommit`; `RemoteRows { AwaitingProbe, Listed, Unreadable }`). `HeadRelation` gains `Unreadable` and `AwaitingRepoProbe`; `classify(branch, RepoBranchProbe<'_>, WorktreePlacement)` gives `Linked → Worktree`, `Named(n == branch) → Default`, `NotFound`/other `Named` → `Feature`, `Unreadable → Unreadable`, `Awaiting → AwaitingRepoProbe` (bare name). The Branch row reads `{name} · unreadable`; `GitCheckout::head_value_is_unreadable()` covers it and `UnreadableHead`. `DetailField::RemotesUnreadable` replaces the remotes table when the listing failed. `field_value_style(data, field)` (`git/render.rs`) is one `match` producing a private `GitValueStyle { Plain, Error, Warning, Success, Inactive, Broken }`; its first arm is `data.failed_reads.field_failed(field) → Error`. Remote tracked and sync cells and worktree comparisons draw `unreadable` in `error_color()`. Bisect reads `bisecting — progress unreadable`.
- Project list: `build_row_cells` matches `(WorktreeHealth, GitStatus)`; a broken row shows no git icon (phase 12's on-error style wins), a normal unreadable row shows `⛔` and an `error_color()` name.
- CI: `CiApplicability { Applies, Unpublished, UnreadableEvidence }` with `of(CheckoutProbe<'_>, RepoBranchProbe<'_>)` (`tui/state/ci.rs`) replaces `unpublished_ci_branch_name` and `is_unpublished_branch`; detached and unborn checkouts apply; an unreadable HEAD, upstream or default branch is `UnreadableEvidence`, which hides runs and the branch toggle. The Package pane Ci row shows `CiDisplay::Unreadable`. The CI Runs pane uses private `CiRemoteAccess { Awaiting, Github, Other, Unreadable }`; unreadable remotes or evidence give `CiEmptyState::Unreadable`, title `CI Runs — unreadable`, runs hidden.
- Package pane: `PackageVersion { AwaitingMetadata, Manifest(String), Submodule(PinnedCommit) }`; an unreadable pin renders in `error_color()`.
- Disk cell: `DiskCell { Size(String), Broken(String), DeleteMarked }` (`tui/panes/constants.rs`, built by `DiskCell::for_row(size, RowLifecycle, WorktreeHealth)`) owns text, suffix and style; every render site and every `observe_disk_width` call reads it, so `DISMISS_SUFFIX` (`" [x]"`, `pub(in crate::tui)`, re-exported `cfg(test)`) is measured.
- Tests: `git_output_logged_fails_on_a_non_zero_exit`, `existence_checks_tell_absent_from_unreadable`, `git_status_of_an_unreadable_checkout_is_unreadable_not_clean`, `value_reads_of_an_unreadable_checkout_are_unreadable`, `primary_head_failure_is_unreadable_but_unborn_is_not_compared`, `corrupt_bisect_refs_report_unreadable_progress`, `submodule_pins_distinguish_unreadable_from_unborn_parent`, `branch_relation_with_unreadable_default_branch_is_unreadable_not_feature`, and in `tui/app/mod.rs` `selecting_an_unreadable_checkout_renders_unreadable_rows`, `corrupt_remote_head_renders_unreadable_branch_and_marks_project`, `broken_worktree_row_with_unreadable_status_draws_no_unreadable_icon`, `unreadable_upstream_or_default_branch_hides_parent_ci`, `unreadable_branch_evidence_hides_cached_runs_in_ci_pane`, `repository_without_workflows_keeps_no_workflow_when_head_is_unreadable`, `delete_marked_root_and_member_draw_whole_disk_suffix_at_160_columns`. Drawn text is found through `find_text_cell` / `find_text_cell_in_row` and the constants, never literals; a corrupt checkout is `.git/HEAD` = `garbage`.

**Files:**
- `crates/cargo-port/src/project/git/{command,branches,checkout,submodule,constants}.rs`, `repo/{info,remote,push,history}.rs` — runner, typed probe outcomes, `GIT_FORMAT_UPSTREAM_SHORT_ARG`
- `crates/cargo-port/src/constants.rs` — `GIT_STATUS_UNREADABLE`, `GIT_UNREADABLE_LABEL`
- `crates/cargo-port/src/tui/panes/pane_data/formatting.rs` — `GitReadFailures`, `GitReadHealth`, `GitSyncLabels`, ahead/behind formatters over `Comparison`
- `crates/cargo-port/src/tui/panes/pane_data/{git_data,detail_field,detail_data,copy_data,package_data}.rs` — `GitProbe`, `RemoteRows`, `HeadRelation::classify`, `RemotesUnreadable`, `PackageVersion`
- `crates/cargo-port/src/tui/panes/git/render.rs` — `GitValueStyle`, `field_value_style`
- `crates/cargo-port/src/tui/state/ci.rs`, `tui/panes/ci/data.rs`, `tui/panes/package/render.rs`, `tui/project_list/list.rs` — `CiApplicability`, `CiRemoteAccess`, `CiEmptyState::Unreadable`, `CiDisplay::Unreadable`, list markers
- `crates/cargo-port/src/tui/panes/constants.rs`, `widths.rs`, `project_list/tree_rows.rs`, `tui/columns/project.rs` — `DiskCell`, Disk fit, `⛔` icon and broken-row match
- `crates/cargo-port/src/tui/app/mod.rs` — render tests, `find_text_cell_in_row`
- `crates/cargo-port/CHANGELOG.md` — `[Unreleased]` `Fixed` lines

**Gotchas:**
- A new failed read must be added to `GitReadFailures` with its owning row, or the list `⛔` and the Git pane rows disagree.
- The CI Runs pane orders access reasons (unreadable, requires GitHub origin) before workflow presence; the Package pane CI row puts `NoWorkflow` first.
- `git_command` stays private to `command.rs`; a direct spawn elsewhere reintroduces reading a failed exit as an answer.

**Ruled out:** reordering the CI Runs pane so a project without workflows says "no workflow" before "unreadable" — the access-first order stays.

### Phase 15 — cargo-port's project list never cuts off a broken worktree's ` [broken]` suffix  · status: done

#### As-built

- The Disk column's fitted width is the drawn size text plus the row's suffix: `observe_disk_width(widths: &mut ProjectListWidths, disk: &str, health: WorktreeHealth)` in `widths.rs` adds `BROKEN_WORKTREE_DISK_SUFFIX` (` [broken]`) on `WorktreeHealth::Broken` through an exhaustive match. It takes the drawn `formatted_disk(bytes)` text and serves the root, submodule and both worktree-entry measures; member and vendored rows, which never draw the suffix, measure the size text alone. At 160 columns a broken row reads ` 304 B [broken]`, white on the error red.
- The Name column's fitted width equals the drawn prefix plus label for every row kind. `TreeRowExpansion { Expanded, Collapsed, Leaf }::prefix(has_later_sibling) -> &'static str` is the one ending text `tree_prefix` draws; `TreeRowDepth` (`root_child`, `child`, `member_of(&MemberGroup)`, `from_lineage_length`, `ancestor_indices`) is the depth both fit and render use; `tree_prefix_width(TreeRowDepth)` reserves the widest ending (4 cells) plus 2 cells per ancestor, independent of expansion and sibling order. The per-kind `PREFIX_*` width constants are gone; `PREFIX_WORKTREE_FLAT` is `"└── "`.
- A flattened worktree group (`renders_as_group() == false`) fits its single live checkout at root depth through `observe_rust_children_fit_widths`.
- `vendored_row_label(name)` (`"{name} (v)"`) feeds the vendored fit and all three vendored renderers.
- The watcher test `spawn_watcher_thread_keeps_watcher_guard_alive_until_shutdown` builds its client with `HttpClient::new_without_github_auth_for_test`, skipping the `gh auth token` subprocess (1.308 s → 0.029 s in the full run), every assertion kept.

**Files:**
- `crates/cargo-port/src/tui/panes/constants.rs` — `BROKEN_WORKTREE_DISK_SUFFIX` (`pub(in crate::tui)`), `vendored_row_label`, `TreeRowExpansion`, `TreeRowDepth`, `tree_prefix_width`
- `crates/cargo-port/src/tui/panes/widths.rs` — `observe_disk_width`, depth-based Name fit, flattened-group branch, module doc
- `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs` — `tree_prefix` through `TreeRowExpansion`/`TreeRowDepth`; `disk_suffix_for_state` draws the suffix constant; vendored label
- `crates/cargo-port/src/tui/panes/mod.rs` — `#[cfg(test)]` re-export of `BROKEN_WORKTREE_DISK_SUFFIX`
- `crates/cargo-port/src/tui/app/mod.rs` — `broken_worktree_disk_suffix_is_drawn_whole_at_160_columns` (real frame, dark and light); `broken_worktree_project_row_draws_on_error_text_on_error_red` requires every suffix cell; Name-fit tests `linked_package_worktree_name_fit_matches_rendered_prefix`, `flattened_worktree_member_name_fit_matches_rendered_prefix`, `vendored_name_fit_matches_rendered_label` and helper `assert_name_fit_matches_frame`
- `crates/cargo-port/src/watcher/runtime.rs` — the watcher test's auth-free client
- `crates/cargo-port/CHANGELOG.md` — `[Unreleased]` Fixed line for the suffix

**Binds later work:** Fit and draw read one text: a row's prefix comes from `TreeRowExpansion::prefix`, its depth from `TreeRowDepth`, a vendored label from `vendored_row_label`, the broken suffix from `BROKEN_WORKTREE_DISK_SUFFIX`; a new row kind or label measures through these, never a parallel constant. Tests reach a `tui::panes::constants` item through a `#[cfg(test)]` re-export in `tui/panes/mod.rs`, never a literal.

**Gotchas:**
- `TreeRowDepth::from_lineage_length` computes `length - 2` and underflows below 2; `tree_prefix` returns the root prefix before calling it.
- A deleted row's ` [x]` Disk suffix is not counted in the Disk width, so a list whose widest size is under 7 columns cuts it.
- `verify.sh check|lint cargo-port` compile, mend and format the whole workspace, so uncommitted work in another crate (cargo-berth) fails them.

**Ruled out:** a duplicate constants module, `include!`, or source parsing to reach the private suffix from tests (the `cfg(test)` re-export instead); per-expansion-state prefix widths in the fit (the fit would change with expansion); raising any slow-test limit for the watcher test.

### Phase 16 — The full cargo-berth suite passes on the Mac, and every test runs under a second on natedev  · status: done

#### Work Order

**Depends on:** phase 11 (done, checkpoint e000cbbe).

**Goal:** A cold `cargo nextest run -p cargo-berth` on the Mac passes every test. On natedev, quiet and cold, every test runs in under 1 s, the three split_checkpoint tests included (showrunner, 2026-10-18: the third phase in a row whose sub-second goal shipped unmet). A cold run of one integration target alone (`cargo nextest run -p cargo-berth --test <target>`) passes.

**Decision (showrunner, 2026-10-18, after four repair rounds):** the Mac "every test under 1 s" goal moves whole into phase 18, run next. The Mac stayed saturated for the whole cold run (load 2.8 → 12.5 on 12 cores) and its total held at 93.5 s while per-test cuts moved individual tests (for example 5.2 s → 1.8 s); with the machine saturated, a per-test cut only moves time between tests, so reducing total work is the only lever. Phase 16 checkpoints when natedev is back under 1 s for every test, the CI timings are measured against the 0.7 s bar (numbers and the cause of each test over it sent to the showrunner), and the Mac-only fix is in. Every cargo-berth test in CI runs under 0.7 s (showrunner, 2026-10-18: phase 11's target, shipped unmet; CI run 39104026332 had 46 over it, max 1.265 s). Every branch push runs CI, so a CI run of this branch measures it before the checkpoint.

**Spec:**
1. Source: the showrunner, 2026-10-18 (verbatim): "The Mac slowness gets its own item, current. Two cargo-berth tests fail there, so it is a wrong result, not just slow. Goal: the full cargo-berth suite passes on the Mac and every test there runs under 1 s. Cause first, measured before any cut. Candidates to rule in or out: TMPDIR is unset over ssh, so tempdirs land in /private/tmp. State which route each run used. macOS scans each freshly written executable or hook script on first exec. Per-spawn cost of git and of the hooks. Fix at the cause, never by raising a limit. Record before and after times on both machines."
2. Baseline, measured 2026-10-18:
   - **Mac**, 00:4x. Route: `ssh mac zsh -lc`, `TMPDIR` set to `getconf DARWIN_USER_TEMP_DIR` (`/var/folders/rf/twhh0jfd243fpltn5k0w1t980000gn/T/`, which sits under the `/var` → `/private/var` symlink). Staged tree `~/rust/scratch/berth-phase7-timing`, cold template cache, load 2.7 on 12 cores, nextest's default 12 threads. Result: 1139 run, 1137 pass, 2 FAIL; `SETUP PASS` 4.735 s; total 95 s. Log `mac:~/rust/scratch/p11_timing.log`, run script `mac:~/rust/scratch/run_timing_p11.sh`.
   - The two failures are `edges successor_board_has_fixed_cold_cost` (1.119 s) and `edges successor_round_robin_covers_every_head` (1.153 s). Both panic at `crates/cargo-berth/tests/edges.rs:2564` with `linked administration should be in common git: StripPrefixError(())`, not on a time bound. That line strips `root.join(".git")` from the path `linked_worktree_administrative_directory` (`edges.rs:2764`) reads out of the linked worktree's `.git` pointer. Git writes that pointer as a resolved path, so a `/var` vs `/private/var` (or `/tmp` vs `/private/tmp`) spelling difference is the first suspect. Spec 4 confirms or rules it out.
   - Slowest on the Mac: liveness `direct_release_waits_for_a_cover_at_a_present_integration_target` 9.459 s, liveness `orphan_alert_and_manual_recovery_use_the_reservation_target` 8.234 s, liveness `release_uses_trunk_after_the_reservation_target_disappears` 7.874 s, lifecycle `upper_checkpoint_keeps_the_full_split_interval_when_update_refs_writes_lower_first` 6.798 s. Most tests take 5–10x their natedev time, e.g. `rewritten_integration_replacement_validates_evidence` natedev 0.225 s / Mac 5.097 s; `orphaned_middle_predecessor_recovers_without_losing_its_outgoing_edge` 0.210 s / 2.385 s.
   - **natedev**, same tree: `verify.sh`, 6 threads, load 22–30, **warm** templates: 1138/1138 pass in 27 s. This is warm and the Mac run was cold, so Spec 4 first takes a cold natedev baseline (delete `target/tmp/cargo-berth-templates/`) to compare against.
3. Every run names its route in its record: machine; how it was launched (`ssh mac zsh -lc …`, or `verify.sh` and which host it picked); `TMPDIR` value and its resolved path (`realpath`); staged tree path; template cache cold or warm; thread count; load and core count at start. A run at load above the core count is repeated, not trusted. Run one suite at a time on each machine.
4. **(a) Cause before any code change.** Run each candidate as an experiment on the Mac with exactly one variable changed against the Spec 2 Mac baseline. Write every result to `phase16-mac-timing.md` in the delegate session directory, one row per run with the Spec 3 route fields. The experiments:
   - **TMPDIR route.** Three cold full runs of the same staged tree: `TMPDIR` unset (tempdirs land in `/tmp` → `/private/tmp`); `TMPDIR=$(getconf DARWIN_USER_TEMP_DIR)` (the baseline); `TMPDIR` set to a directory with no symlink in its path under `~/rust/scratch/` (`realpath` equals the path). Record for each: whether the two Spec 2 failures occur, the panic text, and the times of the Spec 2 slowest tests. This decides whether the failures follow the temp root's spelling.
   - **First-exec scan.** On the Mac and natedev, time the first exec of a freshly written `#!/bin/sh` script (written, `chmod 755`) against its second and tenth exec, over at least 20 fresh scripts, and record `xattr -l` on one of them. Then time a hooked `git commit` in a fresh template copy whose hook scripts were written by that copy, against one whose hooks directory was executed once before (pre-warmed), and against one whose hook scripts are ad-hoc signed (`codesign -s -`). Record whether per-copy hook writes cost the first exec.
   - **Per-spawn cost.** On both machines, time 200 bare `git --version`, 200 `git rev-parse HEAD` in a fixture repository, 200 `sh -c :`, 50 `cargo-berth --version` (the dev build the tests run), and 50 hooked `git commit`s against 50 with `-c core.hooksPath=/dev/null`. Give per-spawn ms and the Mac/natedev ratio. With these counts of git starts per step (a hooked lane commit 2; a hooked commit on `main` or a gated target 3; `merge --ff-only`, `merge --no-ff` or `reset --hard` on a gated target 2; a lane ref change 1; a rebase 1 per picked commit plus 1 per updated branch), estimate the start count of the four slowest tests and check it against their Mac time.
   - **fsync.** Added by the plan author, not in the ruling: Rust's `File::sync_all` on macOS issues `F_FULLFSYNC`, which flushes the drive cache, and product code calls `sync_all`/`sync_data` in `ledger/journal.rs`, `ledger/handle.rs`, `ledger/compaction.rs`, `gate/permit.rs`, `gate/install.rs` and others. Time one `sync_all` of a small file on each machine and count the syncs one hooked commit makes (for example with `fs_usage -f filesys` on the Mac). If this is the cause, changing the durability primitive is a product decision: report the measurement to the unit director before changing it.
   - Close Spec 4 with a short verdict in the measurement file: each candidate ruled in or out with the row that decides it, and the share of the Mac slowdown each ruled-in cause accounts for.
5. **(b) The fix where the cause lives.** Test support (`crates/cargo-berth-test-support`, `crates/cargo-berth/tests/support`), the hook install path (`crates/cargo-berth/src/gate/install.rs`), or product code when a product spawn or sync costs it (`crates/cargo-berth/src/git/command.rs` and the sync call sites). Never a raised limit, a longer timeout, a skipped or `#[ignore]`d test, or a macOS-only skip (`cfg(target_os = "macos")` or a nextest filter). For the two failures: if the path spelling is the cause, compare both paths in one resolved spelling at the point where the fixture or helper builds them, and check whether product code derives `worktree_administrative_locator` (`WorktreeAdministrativeLocator`) the same way; if it does, the fix goes there too. A fix may not set `TMPDIR` for the suite as a way around a spelling defect. The Files list holds candidates; the Spec 4 verdict decides which of them change.
6. **(c) The two cold-cost tests.** `successor_board_has_fixed_cold_cost` and `successor_round_robin_covers_every_head` pass on the Mac with every bound unchanged: `assert_merge_observation_budget(&one_argv, 2)` and `(&two_argv, 3)` (`edges.rs:1895-1896`), `POST_COMMIT_ENGINE_GIT_PROCESS_CEILING`, the phase 8 cold-cost bounds, and `tests/support/timing.rs` (`POLL_INTERVAL`, `SCHEDULING_ALLOWANCE`).
7. **(d) Timing proof.**
   - Mac: one full cold `cargo nextest run --no-fail-fast -p cargo-berth` over `ssh mac zsh -lc` against a staged copy outside `/tmp` (rsync of the worktree, excluding `target/` and `.git/`), after `rm -rf target/tmp/cargo-berth-templates`, waiting for load below a quarter of the cores as `run_timing_p11.sh` does. Every test passes. Record its times and the `SETUP PASS` line; the under-1 s goal there is phase 18's.
   - natedev: one full cold run (`target/tmp/cargo-berth-templates/` deleted) with `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth --local "phase 16: natedev timing record the showrunner's ruling asks for"`. Every test passes, and no test is slower than its Spec 4 cold natedev baseline beyond run-to-run noise (state the noise from the two baseline runs).
   - Before/after tables for both machines in the phase report: the four Spec 2 slowest tests, the two examples, the two failing tests, `split_checkpoint_survives_an_active_phases_rewritten_retention_ref`, `split_checkpoint_survives_stacked_branches_rewritten_with_update_refs`, `split_checkpoint_survives_switching_the_issuing_checkout_before_board`, the run total and `SETUP PASS`, each row naming its route (Spec 3).
   - Never time a test alone or with a `--filter` subset (phase 8: those times did not hold under a full run).
8. **(e) A cold single-target run passes** (phase 11 review F005, moved here by the showrunner). Today a cold `cargo nextest run -p cargo-berth --test <one target>` lists only that target, so `recorded_templates --list` never runs, `builder-run-id` is never written, and `setup-recorded-templates.sh` skips the builder; the target then builds every template inside its tests, slowly, or fails when it needs a template only the builder makes. Make the setup build the templates whenever the cache does not hold them for this run, whichever targets are listed, so a target-selected cold run passes with the same per-test times as a full run. Add a test in `crates/cargo-berth/tests/recorded_templates.rs` that proves it: from an empty template directory, the setup path a single-target run takes leaves every recorded template built.
9. **Phase 14 is checkpointed** (`30d5cc01`), so `crates/cargo-port/` compiles and `verify.sh lint cargo-berth` covers the workspace cleanly; take the lint gate after this phase's last edit. Edit nothing in `crates/cargo-port/`. Run one `verify.sh` at a time in this worktree.

**Files:**
- `crates/cargo-berth/tests/edges.rs` — `:2564` strip-prefix and `linked_worktree_administrative_directory` (`:2764`): the two Mac failures
- `crates/cargo-berth/tests/support/linked_checkout.rs` — `administrative_directory`, the shared form of the same pointer read (`liveness.rs:2269` uses it)
- `crates/cargo-berth-test-support/src/repository_template.rs` — `instantiate` (`:151`, `:194`; canonicalizes the copy root at `:198`): each test's copy writes every hook script fresh, and sets the root's spelling
- `crates/cargo-berth-test-support/src/directory_snapshot.rs` — the per-test file copy, which writes the executables and hook scripts each test runs
- `crates/cargo-berth-test-support/src/integration_repository.rs` — fixture worktree creation and its canonical paths (`:205`, `:651`)
- `.config/nextest.toml` — add `nextest-version = { required = "0.9.138" }` (absent at HEAD), so an older nextest refuses the run instead of skipping the template builder
- `crates/cargo-berth/tests/support/reanchoring.rs`, `crates/cargo-berth/tests/support/reanchoring_templates.rs`, `crates/cargo-berth/tests/support/split_rebase.rs` — the editor executables and hooks these fixtures write fresh per test, if the first-exec cost reaches them
- `crates/cargo-berth/tests/gate.rs`, `crates/cargo-berth/tests/drift.rs` — tests that write a hook in place unlink it first, so a shared pre-executed hook inode is never overwritten
- `crates/cargo-berth/tests/setup-recorded-templates.sh` — Spec 8: build the templates on a cold single-target run
- `crates/cargo-berth/tests/recorded_templates.rs` — Spec 8: the builder and the test that proves a single-target cold run
- `crates/cargo-berth-test-support/src/git_driver.rs` — `fixture_git_command` (`:42`), `git_command` (`:69`), `run_without_hooks` (`:196`), `add_worktree_without_hooks` (`:230`): every fixture git spawn
- `crates/cargo-berth-test-support/src/berth_command.rs` — `berth_command` (`:23`): every `cargo-berth` start a test makes
- `crates/cargo-berth/tests/support/git_configuration.rs`, `crates/cargo-berth/tests/support/loose_refs.rs` — phase 8's spawn-free config and ref reads, extended if spawn cost is ruled in
- `crates/cargo-berth/src/gate/install.rs` — `install_managed_hook` (`:197`) and `PendingManagedHookReplacement::activate` (`:315`): write each hook script at `0o755` and sync the hooks directory (`:224`, `:270`)
- `crates/cargo-berth/src/git/command.rs` — `git_command` (`:42`), `git_command_with_hook_execution_policy` (`:47`): every product git spawn
- `crates/cargo-berth/src/ledger/journal.rs`, `crates/cargo-berth/src/ledger/handle.rs`, `crates/cargo-berth/src/gate/permit.rs` — `sync_all` call sites, only if the fsync experiment rules it in and the unit director approves

**Workers:** 1 writer. Nothing splits: the measurement decides which files change, and one seat must hold both machines' runs.
- `mac` (opens as `impl`) — Spec 4's measurement first, then the fix in every file the verdict names, lint, and both machines' Spec 7 runs. Mac runs go over `ssh mac zsh -lc` against a staged copy outside `/tmp` (rsync of the worktree excluding `target/` and `.git/`), and every run names its route (Spec 3).

**Constraints from prior phases:** see Delegation Context. Phase 7: templates live under `CARGO_TARGET_TMPDIR/cargo-berth-templates/`; deleting that directory forces a cold build. Phase 8: fixture git configuration is written without spawning git (`tests/support/git_configuration.rs`) and loose refs are read directly (`tests/support/loose_refs.rs`). Phase 11 builds `BeforeRun` recorded templates in a nextest setup script, `crates/cargo-berth/tests/setup-recorded-templates.sh`, which runs the builder only when `${CARGO_TARGET_DIR:-target}/tmp/cargo-berth-templates/builder-run-id` (written by `recorded_templates --list`) matches `NEXTEST_RUN_ID`; nextest sets that variable while listing only from 0.9.138, so an older nextest skips the builder and every test builds its own templates. Natedev runs cargo-nextest 0.9.143; the Mac ran 0.9.136 from nix until 0.9.143 went into its `~/.cargo/bin` on 2026-10-18. A Spec 7 run counts only when its `SETUP PASS` line names the wrapper and includes the builder's time (natedev cold 2.3–2.9 s). Phase 11 left three split_checkpoint tests over target on natedev cold (2.102 s, 2.587 s, 1.849 s), which the showrunner assigned to this phase. `POST_COMMIT_ENGINE_GIT_PROCESS_CEILING` and the cold-cost bounds are unchanged.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` pass. `phase16-mac-timing.md` holds a measured verdict for every Spec 4 candidate, recorded before the first code change, each row naming its route. The Spec 7 cold Mac run passes every test, `successor_board_has_fixed_cold_cost`, `successor_round_robin_covers_every_head` and `inherited_git_config_overlay_survives_hook_suppression` included; the Spec 7 cold natedev run, quiet, passes with every test under 1.000 s and no regression past the stated noise. A cold `cargo nextest run -p cargo-berth --test edges` (template directory deleted first) passes on natedev, and the Spec 8 test passes. A CI run of this branch's tip (pushed by the unit director on request) is measured against 0.700 s per cargo-berth test, and its numbers, with the cause of every test over the bar, go to the showrunner. The diff raises no limit, adds no `#[ignore]`, skip, `cfg(target_os = "macos")` branch or nextest filter, and leaves the Spec 6 bounds unchanged. The phase report carries before/after tables for both machines.

### Phase 17 — Drift never widens one phase's reservation with another phase's files, committed ones included  · status: done

#### As-built

A full or cheap `cargo-berth drift` for one reservation never adds a path that another reservation of the same worktree and run holds, or a path that reached the comparison only through that sibling's checkpoint commit, while the sibling is `Outstanding` or `Released`.

- `drift/widening.rs` holds the admission. `WideningAdmission::new(reservations, subject, changes)` is built once per subject beside `subject_coverage_probe`; it sets a private `ComparisonEnd::{SiblingCheckpoint, OwnRange}` to `SiblingCheckpoint` when another reservation with `has_coordination_identity(subject.worktree, subject.run)` has a `RetainedProtectedTip::Retained` tip that `ObservedDriftChanges::compares_to_checkpoint` matches.
- `WideningAdmission::admit(&self, path, coverage) -> WideningPathAdmission::{Excluded, Admitted(UnheldWideningPath)}` admits only a `DriftBlockingCoverage::Unclaimed` path that `carries_work`. A `committed_only` path is also excluded under `SiblingCheckpoint`, and under `OwnRange` unless `head_commit_introduced`. A path the subject dirties again after a sibling's checkpoint still widens the subject.
- `UnheldWideningPath(ReservationScopePath)` has no other constructor; `DriftEffectBuilder::widened_paths` is `Vec<UnheldWideningPath>`, the only route to `JournalOperation::Widen`. `released_sibling_checkpoint_covers` and its race-scope term are gone: every path a sibling's checkpoint commit introduced counts as the sibling's work, inside its declared scopes or not.
- `Reservation::protected_checkpoint(&self) -> &RetainedProtectedTip` reads `retained_protected_tip`, which stays retained through `Outstanding` and `Released`, so the sibling test reads the checkpoint fact instead of a lifecycle variant.
- `protection_in_worktree` still answers `Clear` for a non-`Active` holder in its own worktree; hooks and claims depend on that.

**Files:**
- `crates/cargo-berth/src/drift/widening.rs` — `WideningAdmission`, `ComparisonEnd`, `WideningPathAdmission`, `UnheldWideningPath`
- `crates/cargo-berth/src/drift/classification.rs` — `classify_locked` `Unclaimed` arm calls `admit`; `widened_paths: Vec<UnheldWideningPath>`
- `crates/cargo-berth/src/drift/ordering.rs` — `normalize_paths<Path: Borrow<ReservationScopePath>>`, shared by widened and collision paths
- `crates/cargo-berth/src/reservation/{record,retention,mod}.rs` — `protected_checkpoint`; `RetainedProtectedTip` is `pub(crate)` and re-exported
- `crates/cargo-berth/tests/parallel_phases.rs` — `a_phase_checkpoint_does_not_adopt_an_outstanding_siblings_committed_paths` (linked worktree, A checkpointed and `outstanding`, B widens with `b-extra.rs` only, never `a.rs`) and `a_cheap_drift_for_one_phase_leaves_its_siblings_held_paths_alone` (`cheap_delta` widens A with `a-extra.rs` only; B keeps `[b.rs]`)
- `crates/cargo-berth/CHANGELOG.md` — `[Unreleased]` `Fixed` entry

**Gotchas:** The older `a_phase_checkpoint_does_not_adopt_a_released_siblings_committed_paths` runs on `main`, where reconciliation settles the sibling to `Released` before the drift, so it never exercised the `Outstanding` case; the linked-worktree test does. An explicit `--reservation` drift still widens the named reservation with moved paths no reservation holds, so a phase whose workers write before its claim exists picks up those paths; that is a phase-open matter outside cargo-berth.

### Phase 18 — The cargo-berth suite starts few enough processes that every test runs under a second on the Mac  · status: todo

#### Work Order

**Depends on:** phase 16 (its checkpoint carries the Mac-only fix, the canonical `/private/var` template relocation, and the per-family fixture cuts this phase measures from). Runs next, right after phase 16.

**Goal:** A cold full `cargo nextest run -p cargo-berth` on the Mac passes every test, and every test there runs in under 1 s. It gets there by cutting the total number of processes the suite starts — git and cargo-berth — with product code cut first, where cargo-berth itself starts git. Natedev stays under 1 s for every test.

**Spec:**
1. Source: the showrunner, 2026-10-18 (verbatim): "The Mac \"every test under 1 s\" goal moves whole into a new phase, numbered right after 16 and run next, not left for later. It is about process starts across the whole suite: measure starts per test; set a budget for the suite; cut where most starts come from, in product code first (where cargo-berth itself starts git). A test-side workaround is not the fix. Its exit criterion is the Mac goal: every test under 1 s on a cold full Mac run." And: "when the Mac is saturated and the total does not drop, per-test cuts only move time between tests. Reducing total work is the only lever."
2. Baseline from phase 16, 2026-10-18. **Mac**, cold full run of phase 16's round 4 tree: `ssh mac zsh -lc`, `TMPDIR` = `DARWIN_USER_TEMP_DIR`, 12 threads, load 2.79 → 12.45 on 12 cores. 1146/1146 pass; 356 tests at 1 s or more and 487 at 0.7 s or more; slowest `board cold_proof_subjects_bound_git_evaluation_for_distinct_and_duplicate_reservations` 5.733 s; SETUP 11.989 s; total 93.550 s, against 93.081 s before phase 16's cuts. Phase 16's Mac trace (`phase16-mac-timing.md` in that phase's session directory, R13) attributes the cost to process starts at suite concurrency: board cold proof 163 git starts, lifecycle integration cover 170, edges witness and predecessor 189 git and 16 cargo-berth starts each, and 3029 cargo-berth starts across the suite. fsync time was a 0–6 % share and is not this phase's lever.
3. **(a) Measure starts per test, before any cut.** For every test in a full run, count the processes it starts, split by origin: fixture git (`cargo-berth-test-support` `git_driver.rs`), `cargo-berth` starts (`berth_command.rs`), product git started by cargo-berth (`src/git/command.rs` `git_command`, `git_command_with_hook_execution_policy`), and git or cargo-berth started by installed hooks (`src/gate/install.rs`). Counts do not depend on the machine, so natedev is enough for them. Write `phase18-process-starts.md` in the session directory: the suite total by origin, the top 20 tests by total starts, and the top 20 product call sites by starts across the suite (call site = the cargo-berth function that built the git command, with its argv shape).
4. **(b) The suite budget.** From the Mac's measured per-start cost (phase 16 Spec 4 per-spawn rows) and its 12 threads, derive the total start count at which a cold full Mac run keeps every test under 1 s, with headroom. Record it in the measurement file. Pin it with a test that fails when the per-operation start counts it rests on grow — the same form as `POST_COMMIT_ENGINE_GIT_PROCESS_CEILING` and phase 8's cold-cost bounds — so a later change cannot quietly spend it.
5. **(c) Cut product starts first.** At the call sites Spec 3 ranks highest, make cargo-berth start fewer git processes per operation: read refs, config and objects without a process where phase 8's test-side readers show it can be done (`tests/support/git_configuration.rs`, `tests/support/loose_refs.rs`), batch repeated reads into one process (`git cat-file --batch`, `for-each-ref`), and stop repeating a read whose answer the operation already holds. Product behavior is unchanged; every existing test passes with its assertions intact. Adding a dependency (for example an in-process git library) is a design choice: send it to the showrunner before adding it.
6. **(d) Test-side starts only after (c).** Fixture starts may be cut only where Spec 3 shows them dominant after the product cuts, and only by doing less work (recording a template once, as phase 11 does), never by hiding work from the clock or the count.
7. **(e) Timing proof.** One cold full Mac run (templates deleted, staged copy outside `/tmp`, route recorded per phase 16 Spec 3) passes every test with each under 1.000 s, and its total drops below phase 16's 93 s in proportion to the starts cut. One cold quiet natedev `verify.sh test cargo-berth --local "<why>" --no-cache` passes with every test under 1.000 s. A CI run of the tip is measured against 0.700 s per test; its numbers and the cause of any test over that bar go to the showrunner. Before/after table per machine: the Spec 2 slowest tests, run total, SETUP, and the suite's start count by origin.

**Files:** Spec 3 decides which change.
- `crates/cargo-berth/src/git/command.rs` — `git_command`, `git_command_with_hook_execution_policy`: every product git start, and where per-origin counting can hook in
- `crates/cargo-berth/src/gate/install.rs` — installed hooks and what they start per commit
- the cargo-berth modules Spec 3's top product call sites name (board evaluation, drift, release, integration cover, edges) — added to the phase's files list as Spec 3 names them
- `crates/cargo-berth-test-support/src/git_driver.rs`, `crates/cargo-berth-test-support/src/berth_command.rs` — fixture and cargo-berth start counting
- the Spec 4 budget test, beside the existing cold-cost bounds

**Workers:** 1 writer, `mac`. One seat holds the measurement, the cuts and both machines' runs.

**Constraints:** No thread-count change, raised limit, longer timeout, skipped or `#[ignore]`d test, `cfg(target_os = "macos")` branch, nextest filter, or suite `TMPDIR` setting. `POST_COMMIT_ENGINE_GIT_PROCESS_CEILING` and the cold-cost bounds may only go down. A timing run uses `--no-cache` and starts with `target/tmp/cargo-berth-templates` deleted. A natedev run at a load above 16 of 32 is untrusted. Mac runs: one `mac_test.py claim` per run, taken only when the run is ready to start and released when it ends (hana's standing OK holds until the Mac's screen unlocks; after that, ask hana first). Run one `verify.sh` at a time in this worktree.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` pass. `phase18-process-starts.md` holds the Spec 3 counts, taken before the first cut, and the Spec 4 budget. The Spec 4 budget test passes. The cold full Mac run passes every test with each under 1.000 s; the cold quiet natedev run passes with each under 1.000 s; the CI numbers have gone to the showrunner.
