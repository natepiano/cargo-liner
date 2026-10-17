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

**Depends on:** Phase 8 (shares `crates/cargo-berth/tests/`)

#### Work Order

**Goal:** A `check` or `claim` refused with `blocked_by_overlap` appends exactly one `overlap_refused` journal record: requester in the record's `actor` and `identity_inputs`, the refused paths in `scopes`, and each holder with its overlapping scopes in `holders`. Replay, compaction and the board read the record and never count it as a reservation.

**Spec:**
1. Source: the showrunner, 2026-10-17, asking from natedev for the style pilot. Today a refusal appends nothing. `check` refuses at `verb/check.rs:226-233`, `reconcile_and_retry`'s else branch, from the unlocked `decide` snapshot (`Ledger::read_for_edit_check`, check.rs:291). `check` also refuses at `render_acquisition` (check.rs:266-273), when the locked first-touch retry returns `FirstTouchClaimExecution::Blocked`. `claim` refuses at `validate_claim_transaction` (`verb/claim/mod.rs:1477`, `ClaimRejection::Conflict`) and through `ClaimRejection::IncompleteAnswer`. Both reach `claim_rejection_execution` (~1361), from `acquire_new` (~889-920) and from `named::acquire` (`verb/claim/named.rs:166`). `Ledger::transact_tolerating` (`ledger/handle.rs:731`) handles `TransactionValidation::Reject` at 757-763: it publishes only a required rebuild and appends nothing.
2. **Where the record is written (decided):** at the verb, after the verb has settled on its refusal and before it returns the blocked envelope. The record goes in its own record-only transaction. The verb opens the ledger and calls `Ledger::transact` with a validator that always returns `TransactionValidation::<Infallible>::Append(Box::new(JournalOperation::OverlapRefused { .. }))`. `gate/permit.rs:263-273` writes the bypass audit the same way. `TransactionValidation::Reject` stays append-free. Writing inside the rejecting transaction loses on three counts:
   - The common `check` refusal (226-233) is decided from an unlocked snapshot, so no lock is held there.
   - A first `check` attempt that comes back `Blocked` (check.rs:157) is retried after reconciliation. An in-lock write would record a refusal the caller never received, and the retry could record a second.
   - `FirstTouchClaimRejection::Blocked` (claim/mod.rs:1717, 2090) also serves post-write first touch. Post-write first touch is an incursion, not a refusal.
   Exactly one record is written per `blocked_by_overlap` envelope that `check` or `claim` returns, and none for any other status.
3. **The operation.** `ledger/journal.rs`, inside `declare_journal_operations!` (389-685), after `Answer`:
   ```rust
   /// Record a check or claim refused because foreign reservations hold overlapping paths.
   ///
   /// The record's `actor` and `identity_inputs` name the requester. Replay reads it and
   /// changes no reservation, edge, answer or incident.
   OverlapRefused {
       /// The command whose request was refused.
       verb:    RefusedVerb,
       /// The requested paths the refusal reported.
       scopes:  ReservationScopeSet,
       /// Each foreign holder with the scopes of its own that overlap the request.
       holders: RefusingHolderSet,
   },
   ```
   - `RefusedVerb::{Check, Claim}` has `#[serde(rename_all = "snake_case")]`, so `"verb": "check" | "claim"`. It is a separate two-variant enum, not `output::CommandVerb`, so no other verb can be recorded.
   - `RefusingHolder { reservation_id: ReservationId, scopes: ReservationScopeSet }`: the holder and its `ReservationConflict::overlapping_scopes`. It pairs each holder with its own paths, as `BlockedIncursionPath` (journal.rs:1509) does, so a union of several holders' scopes cannot be represented.
   - `RefusingHolderSet` and `EmptyRefusingHolderSet` come from `nonempty_journal_set!` (1361), next to `ForeignReservationIdSet` (1422).
   - The checked constructor `RefusingHolderSet::from_conflicts(&[ReservationConflict]) -> Result<Self, EmptyRefusingHolderSet>` keeps conflict order. Every refusal path builds its envelope from a non-empty conflict list, so `Err` cannot arise there. The caller appends nothing on `Err`.
   - None of the three new types derives `JsonSchema`, because no envelope carries them.
4. **Requester (decided):** the record's existing `actor` (`repository`, `worktree`, `run`) plus `identity_inputs`, which every new record writes (`JournalMutationIdentityInputs::record`, journal.rs:185: `CARGO_BERTH_SESSION_ID`, `CLAUDE_CODE_SESSION_ID`, `CARGO_BERTH_RUN`, invocation directory). Copying the requester into the operation would let a record name a requester that differs from its own actor. The actor is the `ResolvedJournalMutationActor` (`ledger/authorization.rs:69`) of the refused request:
   - `claim`: `journal_mutation_actor` in `acquire_new` (claim/mod.rs:837), and the matching actor in `named::acquire`.
   - `check`: `resolved_edit_authorization.journal_mutation_actor_for(resolved_edit_authorization.coordination_run_id)`, from `decide` (check.rs:310). An unidentified caller gets a fresh run id per process, because that caller presented no run.
5. **Recording function.** New `crates/cargo-berth/src/verb/refusal.rs` (declared in `verb/mod.rs`): `pub(super) fn record(worktree_context: &WorktreeContext, requester: ResolvedJournalMutationActor, verb: RefusedVerb, scopes: &ReservationScopeSet, conflicts: &[ReservationConflict])`.
   - It opens the ledger with `Ledger::open_from_discovered_worktree` and appends through `Ledger::transact`, which waits up to `MUTATING_VERB_CONTENTION_TOLERANCE` like every mutating verb.
   - It returns nothing. A failed append (lock contention, unreadable ledger, `Err` from `from_conflicts`) leaves the refusal envelope byte-for-byte as today. The requester's remedy does not depend on the record, and hook output stays one accurate refusal.
   - Call sites:
     - `check.rs`: `CheckDecision` (48) gains `requester: ResolvedJournalMutationActor` and `worktree_context: WorktreeContext`, filled in `decide`. The else branch at 226 records `retried_decision.scopes` and `.conflicts`. `render_acquisition` takes the decision's requester and context; its `Blocked` arm (266-273) records `scopes` and `conflicts` before `blocked_check`. The `PostWriteIncursion` arm shares that match arm but cannot occur under `FirstTouchConflictHandling::RefuseRequest`. Split it out so only `Blocked` records.
     - `claim/mod.rs` `acquire_new` and `named.rs` `acquire`: when the execution is `ClaimExecution::Blocked { conflicts, .. }` or `ClaimExecution::IncompleteAnswer { rejection, .. }` (`rejection.conflicts`), record with `RefusedVerb::Claim`. Use the normalized claim `scopes` (claim/mod.rs:840; named.rs:79) and the claim's actor. Record after the refusing transaction returns.
6. **Readers (replay, board, compaction):**
   - `RetainedReservationSet::apply` (`reservation/retention.rs:1032`): add `OverlapRefused` to the no-op arm at 1083-1087.
   - `OrderingGraph::replay` (`edge/graph.rs:155-180`): add it to the no-op list.
   - `JournalOperation::record_ceiling` (journal.rs:750-775): `Declared`, since the request bounds `scopes` and `holders`.
   - `is_coordination_record` (793-826): `false`. Coordination readers never see it, so no board answer, alert, edge or enrollment history reads it.
   - `session::apply_journal_event` (`session/mod.rs:441` wildcard) maps no session.
   - Compaction keeps it (it drops only superseded merge extents).
   - `CURRENT_SCHEMA_VERSION` (`ledger/constants.rs:87`) stays `2`. Precedent: `HolderMergeExtentObserved` was added without a bump (fd71b778).
   - Existing journals decode and replay unchanged, and `tests/fixtures/reader_compat` bytes do not change.
   - The append moves `JournalPosition`, so an open terminal board rebuilds once on its next refresh. Phase 5's rule (no reconcile on an unchanged journal) still holds.
7. **Unit tests** (impl writer, `ledger/journal.rs` `mod tests`):
   - An `overlap_refused` event round-trips through serde in the current schema.
   - `is_coordination_record` is false.
   - Folding a journal of one claim then one `OverlapRefused` naming it yields the same `RetainedReservationSet` as the claim alone.
   - `RefusingHolderSet::from_conflicts(&[])` is `Err`.
8. **Integration tests** (test writer):
   - New `crates/cargo-berth/tests/support/overlap_refusal.rs`, included by `#[path]` from `overlap.rs` and `answers.rs` as `git_configuration.rs` is: `assert_one_refusal_appended(journal_before: &[u8], journal_after: &[u8], verb: &str, envelope: &Value) -> Value`. It asserts:
     - `journal_after` starts with `journal_before` and adds exactly one line.
     - That line's `op` is `"overlap_refused"` and its `verb` is `verb`.
     - Its `holders` reservation ids equal `envelope["blocked_by"]` in order.
     - Each holder's `scopes` equals the matching `envelope["payload"]["data"]["conflicts"][i]["overlapping_scopes"]`.
     It returns the record.
   - New test in `tests/overlap.rs`, `a_refused_claim_journals_its_requester_paths_and_holders`, modeled on `blocked_claim_names_holder_provenance_and_appends_nothing` (334) and using the same `initialized_repository()` template and `foreign_worktree`.
     - Setup: FIRST_RUN claims `tree:crates/hana_kana`. From the second worktree, `claim file:crates/hana_kana/src/lib.rs --run SECOND_RUN --json` exits 1 with `blocked_by_overlap`.
     - Requester: `actor.run == SECOND_RUN`. `actor.worktree` differs from the holder claim record's `actor.worktree`. `identity_inputs.status == "recorded"`.
     - Paths: `scopes == [{"path":"crates/hana_kana/src/lib.rs","kind":"file"}]`.
     - Holder: `holders[0].reservation_id` is the first claim's id, and `holders[0].scopes` equals the envelope conflict's `overlapping_scopes`.
     - Board: the reservation ids across `board --json`'s `ready_now`, `waiting`, `unconstrained_reservations` and `resolved` sections (as `cargo_berth_test_support::reservation_row` walks them) are the same before and after the refusal.
   - Rename `blocked_claim_names_holder_provenance_and_appends_nothing` to `..._and_journals_one_refusal`, and `blocked_check_returns_holder_decision_facts_without_appending` (1200) to `..._and_journals_one_refusal`. Replace each unchanged-journal assertion with `assert_one_refusal_appended`, verb `"claim"` and `"check"` respectively. The check test also asserts `scopes == [{"path":"shared.rs","kind":"file"}]`.
   - In `tests/answers.rs`, these tests assert `journal_bytes(...) == journal_before` after a `blocked_by_overlap` claim:
     - `an_answer_naming_one_holder_is_refused_while_another_holder_conflicts` (~766)
     - `incomplete_before_answer_prints_a_complete_executable_claim` (~799)
     - `incomplete_answer_keeps_the_configured_chooser_after_holder_facts` (~867)
     - `incomplete_defer_answer_prints_a_complete_executable_claim` (~917)
     - `incomplete_named_phase_answer_prints_a_complete_executable_claim` (~991)
     - `incomplete_mixed_answer_prints_a_choice_for_the_missing_holder` (~1085)
     - `released_holders_are_no_longer_answered_and_naming_them_is_refused` (~1369)
     - `a_sequence_answer_naming_a_reservation_that_does_not_conflict_is_refused` (~1433)
     Each becomes `assert_one_refusal_appended(.., "claim", &envelope)`. The no-append assertions on other statuses stay as they are: invalid input (exit 5), `ambiguous_active_run_reservations`, `contention`, `ledger_unreadable`.
   - Any further test the package run shows failing only because a refused check or claim now appends this record gets its expectation updated to include the record. No shared `journal_events` helper filters `overlap_refused` out.
   - Each new or edited test runs under a second.
9. **Wire contract:**
   - No envelope, `--json` payload or `output_contract_version` changes. `docs/cargo-berth/generated/output-contract.json` stays byte-identical, and `generated_artifacts_are_reproducible_from_the_checked_in_contract` stays green without regeneration.
   - The journal gains one operation. An older cargo-berth decodes an `overlap_refused` line as a corrupt interior record, which makes the ledger unreadable. Every machine sharing a repository must run this version before any refusal is recorded. Install on natedev and the Mac together after the checkpoint, under the berth standing approval.
10. **Docs:**
    - `docs/cargo-berth/as-built/json-contract.md` "The journal record" (998): add an `overlap_refused` row (`verb`, `scopes`, `holders`) to the op table (after `answer`, ~1096). Add a tagged-value bullet: `verb` is `check` or `claim`; each `holders` entry is `{ "reservation_id": <uuid-v7>, "scopes": [scope...] }`, non-empty; the requester is the record's `actor` and `identity_inputs`.
    - `docs/cargo-berth/as-built/worktree-coordination.md`:
      - "The journal operation union" (62): add `OverlapRefused` and correct the variant count.
      - "Journal replay" (70): add it to the records read only by the reservation fold, as a no-op.
      - "Claiming and the first-touch path" (251): one sentence saying each refused check or claim appends one record, in its own transaction after the refusal.
    - `crates/cargo-berth/README.md` "Claims and collisions" (246): one paragraph saying every refused `check` or `claim` appends an `overlap_refused` journal record naming the requester, the paths and each holder.
    - `crates/cargo-berth/CHANGELOG.md` `[Unreleased]` `### Added` (10): one entry with the older-binary sentence from item 9, worded like the `replace_release_disposition` entry (~134).

Retention (unit director's call, 2026-10-17): refusal records are kept forever. - Refusal records are kept forever: compaction drops only superseded merge extents, so every refused hook check adds about 0.5 KB to the journal for good. Recommendation: keep them, because the style pilot needs the full count. If journal growth later shows up in replay timings, add a retention rule then (for example, compaction drops refusals older than the pilot window).

**Files:**
- `crates/cargo-berth/src/ledger/journal.rs` — `OverlapRefused`, `RefusedVerb`, `RefusingHolder`, `RefusingHolderSet`, `record_ceiling`, `is_coordination_record`, unit tests
- `crates/cargo-berth/src/ledger/mod.rs` — re-exports of the new types
- `crates/cargo-berth/src/reservation/retention.rs` — no-op arm in `apply`
- `crates/cargo-berth/src/edge/graph.rs` — no-op arm in `OrderingGraph::replay`
- `crates/cargo-berth/src/verb/refusal.rs` — new; `record`
- `crates/cargo-berth/src/verb/mod.rs`
- `crates/cargo-berth/src/verb/check.rs` — `CheckDecision` fields, both refusal sites
- `crates/cargo-berth/src/verb/claim/mod.rs` — claim refusal recording
- `crates/cargo-berth/src/verb/claim/named.rs`
- `crates/cargo-berth/tests/support/overlap_refusal.rs` — new
- `crates/cargo-berth/tests/overlap.rs`
- `crates/cargo-berth/tests/answers.rs`
- `docs/cargo-berth/as-built/json-contract.md`
- `docs/cargo-berth/as-built/worktree-coordination.md`
- `crates/cargo-berth/README.md`
- `crates/cargo-berth/CHANGELOG.md`
- `crates/cargo-berth/tests/` — any further expectation a refusal record changes (owner: `test`)

**Workers:** 1 writer + 1 tester. Product code and docs split from the integration tests. Spec items 3, 8 and 9 fix the wire shape, so the tests can be written before the code exists.
- `impl`: every `src/` file above, the item 7 unit tests, `json-contract.md`, `worktree-coordination.md`, README and CHANGELOG. Hub: `crates/cargo-berth/src/ledger/mod.rs` (re-exports).
- `test`: `tests/support/overlap_refusal.rs`, the new and renamed tests in `tests/overlap.rs`, and the eight `tests/answers.rs` edits from item 8. After `impl` lands, it runs the package tests and updates any further expectation that a refusal record changes, in any `tests/*.rs` file.

**Constraints from prior phases:** see Delegation Context. In particular, phase 5's terminal board rebuilds only on `JournalPosition` movement. A refusal record moves it, and that rebuild is expected.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` pass. The passing tests include:
- `a_refused_claim_journals_its_requester_paths_and_holders`
- both renamed `..._and_journals_one_refusal` tests
- the eight `answers.rs` tests
- the item 7 unit tests
- `generated_artifacts_are_reproducible_from_the_checked_in_contract`, with `output-contract.json` unchanged


### Phase 11 — The 16 cargo-berth tests still at 1 s in CI finish in 0.7 s  · status: todo

**Depends on:** phase 10 (phase 10 owns `crates/cargo-berth/src/` and `crates/cargo-berth/tests/` while it runs; this phase edits `tests/` and `src/ledger/handle.rs`)

#### Work Order

**Goal:** Each of the 16 cargo-berth tests that took 1 s or more in the CI Test Suite run of main ec3297e9 takes 0.7 s or less there. Every test still proves what it proved before.

**Spec:**
1. Source: the showrunner, 2026-10-17: a deeper fixture redesign of the 16 cargo-berth tests still at 1 s or more in CI, target 0.7 s each in CI. Lists: `/tmp/claude/berth_ci_slow_ec3297e9.txt`, `/tmp/claude/berth_ci_times_ec3297e9.txt`. Phase 8's cuts took the slowest from 3.6 s to 1.68 s.
2. Rule (showrunner, verbatim): "measure in a full parallel run like CI's (the whole workspace suite at once), not each test alone. Times taken alone didn't hold under CI's parallel load. A faster test must still prove everything it proved before: keep assertions after reconciliation, and keep the many-scope cases many-scope."
3. The 16 tests (CI seconds):
   - `lifecycle.rs`:
     - `checkpoint_ranges_survive_two_rebases_reconciled_before_fast_forward` 1.684 (`:966`)
     - `checkpoint_ranges_survive_two_rebases_reconciled_during_integration` 1.598 (`:971`)
     - `split_checkpoint_survives_stacked_branches_rewritten_with_update_refs` 1.328 (`:1051`)
     - `split_checkpoint_survives_an_active_phases_rewritten_retention_ref` 1.196 (`:1046`)
     - `upper_checkpoint_keeps_the_full_split_interval_when_update_refs_writes_lower_first` 1.182 (`:1062`)
     - `upper_checkpoint_keeps_the_full_split_interval_when_update_refs_writes_upper_first` 1.127 (`:1056`)
     The bodies of all six are in `tests/support/reanchoring.rs`.
   - `liveness.rs`: `recovery_dispositions_validate_evidence_and_remain_distinct_after_replay` 1.492 (`:1661`).
   - `edges.rs`:
     - `cross_target_edge_keeps_earlier_landed_proof_after_target_revalidation` 1.393 (`:232`)
     - `legacy_negative_successor_verdict_retries_once_under_scoped_contiguity` 1.353 (`:1234`)
     - `cross_target_edge_waits_for_integration_to_land_on_main_then_successor` 1.269 (`:284`)
     - `orphaned_middle_predecessor_recovers_without_losing_its_outgoing_edge` 1.148 (`:2235`)
   - `gate.rs`:
     - `enforce_gate_rejects_cross_target_successor_before_integration_lands_on_main` 1.080 (`:1568`)
     - `enrollment_authorizes_both_edits_but_holds_first_integration_under_observe` 1.078 (`:1633`)
     - `trunk_gate_remembers_first_target_landing_after_target_revalidation` 1.048 (`:1488`)
     - `enrollment_authorizes_both_edits_but_holds_first_integration_under_enforce` 1.017 (`:1643`)
   - `src/ledger/handle.rs` unit test: `ledger::handle::tests::a_derived_record_past_its_own_ceiling_is_not_reported_as_caller_input` 1.065 (`:1585`).
4. Where the time goes, read from the code 2026-10-17:
   - The cost is `cargo-berth` process starts (a dev build; each start reconciles and runs its own git). Most starts come from the managed hooks (`src/gate/install.rs:341,400-566`), not from direct CLI calls.
   - Starts per git step:
     - A hooked commit on a lane: 2 (post-commit `drift --full`, committed `__reference-transaction`).
     - A hooked commit on `main` or a gated target: 3.
     - `merge --ff-only`, `merge --no-ff` or `reset --hard` on a gated target: 2.
     - A lane ref change: 1.
     - A rebase: 1 per picked commit, plus 1 per updated branch.
   - Hook starts outnumber CLI calls in tests 1, 2, 4, 6–9, 11 and 15 (numbered in item 3 order, by CI time).
   - `RepositoryTemplate` refuses a template whose journal holds a record and refuses to relocate paths inside `.git/cargo-berth` (`crates/cargo-berth-test-support/src/repository_template.rs:85-90,138-142`). So every claim / defer / commit / release prefix is replayed live in every test: the two-rebase pair spends about 25 of its 35 starts on an identical `TwoRebaseHistory::new` (`reanchoring.rs:269`).
   - Three tests hold two or more scenarios: `recovery_dispositions_…` (abandon/retire, then rewritten-integration replacement, about 36 starts); `legacy_negative_…` (two `SuccessorGap` cases, each on a fresh fixture); and the enrollment pair (one body shared by four variants, `gate.rs:1654`).
   - The unit test builds 131,072 scope paths (`MAXIMUM_DERIVED_JOURNAL_RECORD_BYTES` = 4 MiB, `src/ledger/constants.rs:47`, divided by 32) and serializes about 10 MB of JSON in a debug build before `append_events` (`src/ledger/journal.rs:2295-2306`) checks the size.
5. **R0 — templates that hold a ledger** (`repository_template.rs`):
   - An opt-in `LedgerFixture::Recorded` lets a template keep journal records and relocate paths inside `.git/cargo-berth`.
   - After the copy, delete `reservations.json` and the replay checkpoint so the next command replays the journal (path lengths change byte offsets and the fingerprint). Renew each linked worktree's `.git/worktrees/*/cargo-berth-worktree-id`.
   - Linked worktrees live under `<root>/.git/fixture-*`, as `REWRITE_MAIN` does, so one root relocates.
   - The builder writes reservation ids and commit tips to a fixture JSON file inside the template, and keeps today's setup assertions, which run on every fresh build.
   - A recorded template older than 12 hours is rebuilt, because reservations turn stale at 24 hours (`src/reservation/record.rs:436`).
   - First confirm with a test that any command (not only `init`) rebuilds a missing projection. If not, R0 runs `init --repair-projection` once per copy.
6. **R1 — rewrite tests** (`tests/support/reanchoring.rs`):
   - New recorded templates:
     - `TWO_REBASE_HISTORY`: the end state of `TwoRebaseHistory::new`, with 2 reservations and pending markers.
     - `STACKED_UPPER_CHECKPOINT`, for both `upper_checkpoint_…` tests.
     - `SPLIT_LOWER_CHECKPOINT_ACTIVE_RETENTION` and `SPLIT_LOWER_CHECKPOINT_UPDATED_SIBLING`, for the two `split_checkpoint_…` tests.
   - Every rebase stays live and hooked (the hooks capture the markers), and so does `drain_markers` (`:2186`). The two-rebase pair's ff to `main` stays hooked: it is the "during integration" case.
   - In the shared drop tail (`assert_dropping_split_preserves_checkpoint`, `:1631`) the ff to `main` runs hookless. If a test still measures over 0.7 s, its drop tail becomes its own test, starting from a template of the drained state.
7. **R2 — cross-target tests**:
   - `IntegrationRepository` (`crates/cargo-berth-test-support/src/integration_repository.rs`) gains a `CROSS_TARGET_PAIR` recorded template: the integration worktree; lane A targeting `integration` and lane X targeting `main`; A claimed, X deferred behind A, A's work committed and released.
   - Edges tests `:232` and `:284` and gate `:1568` start from it. Gate `:1488` uses a `CROSS_TARGET_AFTER_PAIR` variant (`--target integration`, X claimed `--after` A).
   - In `edges.rs`, landing commits and merges run hookless: only board and `sequence` output is asserted.
   - In `gate.rs`, the final `propose_trunk` stays hooked. In `trunk_gate_remembers_…` the landings also stay hooked, because the test is about what the gate remembers.
8. **R3 — `legacy_negative_…`**:
   - It splits into one test per `SuccessorGap` case. Together they keep both cases: many-scope stays many-scope.
   - Each starts from a recorded `REWRITTEN_SUCCESSOR_EQUIVALENT` template (the state of `rewritten_successor_fixture(true)`, `edges.rs:2444`), so its live part is the traced `sequence` and two traced boards.
9. **R4 — `orphaned_middle_…`**:
   - A `LEFT_RIGHT_TREES` template (left and right files committed hookless, plus the six worktrees; no ledger records, so no R0 needed) replaces `commit_left_and_right_trees` (`edges.rs:2862`).
   - The orphaning, recovery, duplicate `sequence` and journal check stay live.
10. **R5 — enrollment**:
    - An `ENROLLED_PAIR` recorded template holds the state up to the board.
    - One live test keeps the `init`, `check`, `drift` and board assertions.
    - The observe and enforce variants (and the two `second_*` variants sharing the body) then run only `set_gate_mode` and `integrate` against it, with `assert_enrollment_gate_decision` (`gate.rs:7590`) unchanged.
    - First check that gate mode does not change `check` or `drift` on lane branches. If it does, build one template per mode.
11. **R6 — liveness**:
    - Split into two tests, `abandon_and_retire_dispositions_remain_distinct_after_replay` and `rewritten_integration_replacement_validates_evidence`. Each keeps its scenario's assertions, including the 2-id loops.
    - In the second, the trunk commits, resets and merge on `main` run hookless. The release command still reconciles first, as the test's comments require.
12. **R7 — unit test**:
    - Build the oversized record from about 64 scope paths of 66 KiB each (`ReservationScopePath` has no length limit) instead of 131,072 short ones.
    - The assertion stays `Err(DerivedRecordTooLarge)`.
    - No product change in this phase: a bounded serializer in `append_events` is a separate change.
13. **What a faster test keeps:**
    - Every assertion each test makes today, including those made after reconciliation (board, `sequence`, drain passes).
    - Every case it covers, many-scope ones included.
    - A setup assertion that moves into a template builder runs on every fresh template build.
    - `POST_COMMIT_ENGINE_GIT_PROCESS_CEILING` and the cold-cost bounds named in phase 8 do not change.
14. Timing measurement, binding:
    - The CI Test Suite nextest log on main after this phase merges. Each of the 16 (and each test split from one of them) reads 0.7 s or less.
    - Local check before the checkpoint: delete `target/tmp/cargo-berth-templates/` (a cold start, as CI's), then one `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` run of the whole package at once, while no other `verify.sh` runs. Read the per-test times of the 16 from that run's nextest output.
    - Never time a test alone or with a `--filter` subset: phase 8 measured that times taken that way did not hold in CI.
    - Record natedev load (`uptime`) beside the run. A run at load above the core count is repeated, not trusted.
15. **Carried from phase 10's review** (round 2 findings, moved here when that phase's gate converged):
    - `acquire_new` (`crates/cargo-berth/src/verb/claim/mod.rs:821`) reads `ClaimRequest` field by field again. Restore the exhaustive destructure, so a new request field cannot be dropped silently, and meet clippy's length limit by extracting a helper, never by an allow.
    - The `LedgerTransactionOutcome::Rejected` arm at `crates/cargo-berth/src/verb/claim/mod.rs:1399` cannot be reached. Make it unrepresentable by type where that transaction cannot reject; otherwise remove the arm's dead handling with the reason in one comment.

**Files:**
- `crates/cargo-berth/src/verb/claim/mod.rs` — Spec 15: `acquire_new` destructure, unreachable `Rejected` arm
- `crates/cargo-berth-test-support/src/repository_template.rs` — R0 `LedgerFixture::Recorded`, projection reset, worktree id renewal, 12-hour rebuild
- `crates/cargo-berth-test-support/src/integration_repository.rs` — `CROSS_TARGET_PAIR`, `CROSS_TARGET_AFTER_PAIR` builders
- `crates/cargo-berth-test-support/src/lib.rs` — re-exports
- `crates/cargo-berth/tests/edges.rs` — cross-target tests, `legacy_negative_…` split, `LEFT_RIGHT_TREES`, `REWRITTEN_SUCCESSOR_EQUIVALENT`
- `crates/cargo-berth/tests/gate.rs` — cross-target gate tests, `ENROLLED_PAIR`, enrollment variants
- `crates/cargo-berth/src/ledger/handle.rs` — R7 unit test
- `crates/cargo-berth/tests/support/reanchoring.rs` — R1 recorded templates, hookless drop-tail ff
- `crates/cargo-berth/tests/lifecycle.rs` — the six rewrite tests, any drop-tail split
- `crates/cargo-berth/tests/liveness.rs` — R6 split, hookless trunk moves
- `crates/cargo-berth/tests/support/split_rebase.rs` — split-rebase helpers the R1 templates reuse

**Workers:** 1 writer + 1 tester, split by file. Both write tests; the redesign is in test code, and the R0 template change sits under both. Of the 16 tests, `impl` takes 9 (4 edges, 4 gate, 1 unit) and `test` takes 7 (6 lifecycle, 1 liveness).
- `impl` — R0, R2, R3, R4, R5, R7: `repository_template.rs`, `integration_repository.rs`, `tests/edges.rs`, `tests/gate.rs`, `src/ledger/handle.rs`. Hub: `crates/cargo-berth-test-support/src/lib.rs` (re-exports). It lands R0 first and tells `test` when `LedgerFixture::Recorded` is in.
- `test` (opens as `test`; lane `crates/cargo-berth/tests/`) — R1 and R6: `tests/support/reanchoring.rs`, `tests/lifecycle.rs`, `tests/liveness.rs`, `tests/support/split_rebase.rs`. Before R0 lands it does the liveness split, the hookless trunk moves and the fixture restructuring that needs no recorded template. It messages `impl` for any `repository_template.rs` line it needs.

**Constraints from prior phases:** see Delegation Context. Phase 7: templates live under `CARGO_TARGET_TMPDIR/cargo-berth-templates/`, one per test binary and executable; deleting that directory forces a cold build. Phase 8: fixture git configuration is written without spawning git (`tests/support/git_configuration.rs`), loose refs are read directly (`tests/support/loose_refs.rs`), and `replaced_deferral_ordering_remains_readable_on_full_and_reservation_boards` keeps its real `claim --defer`, `sequence` and `claim --replace` sequence. Phase 10 makes refused checks and claims append an `overlap_refused` record; a recorded template built after phase 10 may hold them, and nothing here filters them out.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` pass, with every test from item 3 (or its split parts) passing. In the item 14 local run each of them reads 0.7 s or less. The binding check is item 14's CI log after merge; a test still above 0.7 s there goes back to the unit director as a repair.

### Phase 12 — cargo-port's broken-worktree rows in the project list draw white text on the error red  · status: done

**Depends on:** phase 9 (the `on_error` theme token); runs beside phase 10

#### Work Order

**Goal:** A broken linked worktree's row in the project list reads in both themes: its text is drawn in the theme's on-error foreground on the error red, the same style as the Git pane's Error row.

**Spec:**
1. Source: the showrunner, 2026-10-17. Phase 9 added `SemanticTheme::on_error` and `tui_pane::on_error_color()` (white in every built-in theme) and switched the Git pane's Error row to it. The showrunner grepped the workspace: the two sites below are the only other text-on-error-red fills, and both still draw `fg(text_default()).bg(error_color())`, black on `Rgb(170, 0, 0)` in the light theme (about 2.7:1).
2. `crates/cargo-port/src/tui/columns/project.rs:482` — the name-column `broken_style` applied to every span of a `WorktreeHealth::Broken` row — takes `fg(on_error_color())`.
3. `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs:668` — the ` [broken]` disk-column suffix style — takes `fg(on_error_color())`.
4. Nothing else changes: the row's text, the suffix, and which rows count as broken stay as they are.
5. Test: render the project list for a broken worktree entry and read back the styles of the name cell and the ` [broken]` suffix cell from the buffer: foreground `on_error_color()`, background `error_color()`. Under a second.
6. Theme templates (showrunner add-on, 2026-10-17): every shipped theme template — `themes/*.toml` in cargo-port, cargo-tile and cargo-handler, `starter.toml` included — sets `on_error` in its `[semantic]` table, so a user copying a template sees the key. Today they load only through the serde default.
7. Test speed (showrunner add-on, 2026-10-17): `tui::app::tests::worktrees::refreshed_worktree_metadata_appends_into_existing_group` (`crates/cargo-port/src/tui/app/mod.rs:15523`) took 1.107 s; bring it under a second, keeping every assertion. Find where its time goes before cutting.
8. Docs: a `crates/cargo-port/CHANGELOG.md` `[Unreleased]` `Fixed` line.

**Files:**
- `crates/cargo-port/src/tui/columns/project.rs`
- `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs`
- `crates/cargo-port/src/tui/app/mod.rs` — the new test and the item 7 test
- `crates/cargo-port/CHANGELOG.md`
- `crates/cargo-port/themes/`
- `crates/cargo-tile/themes/` — shipped theme templates; owner tile-fixes-unit, edited through berth
- `crates/cargo-handler/themes/` — shipped theme templates; owner handler-enh-unit, edited through berth

**Workers:** 1 writer.
- `impl` or whichever worker phase 10 leaves free — every file above, lint, and the package test.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-port` green, with the item 5 test. Shots: the unit director's dark and light shots of the project list with a broken worktree row show white text on the error red.

### Phase 13 — cargo-port reports a checkout whose HEAD git cannot read as unreadable, never as unborn  · status: todo

**Depends on:** phase 9 (`GitCheckout`, done). Runs beside phases 10 and 12. Shared with phase 12: `crates/cargo-port/src/tui/app/mod.rs` and `crates/cargo-port/CHANGELOG.md`; claim those two paths `--after` phase 12's reservation. Every other file below is phase 13's alone, so the probe and pane work starts at once. Nothing is shared with phase 10 (`crates/cargo-berth/`).

#### Work Order

**Goal:** A checkout whose HEAD git cannot read (a corrupt `.git/HEAD`, a corrupt branch ref) is probed as `HeadState::Unreadable`, reaches the Git pane as `GitCheckout::UnreadableHead`, and shows `HEAD unreadable` in the Branch row in the error color; an unborn repository still shows `unborn`. The Git pane's checkout type has no `Option<HeadRelation>`: a branch always carries its relation, and no other state has one.

**Spec:**
1. Source: the showrunner, 2026-10-17: "an unreadable HEAD reported as Unborn is a wrong result, and the bare `Option<HeadRelation>` is the kind of state the user wants made unrepresentable by type."
2. The defect, measured with git 2.x on natedev 2026-10-17. `command::git_output_logged` (`crates/cargo-port/src/project/git/command.rs:25`) returns `Ok(Output)` for any exit status. `branches::get_current_branch` (`crates/cargo-port/src/project/git/branches.rs:22-33`) ignores the status, and `resolve_head_state` (`crates/cargo-port/src/project/git/checkout.rs:268-292`) maps every failure to `HeadState::Unborn`. `discovery::git_repo_root` (`discovery.rs:53`) only looks for a `.git` entry on disk, so a damaged repository reaches `resolve_head_state`. What `git rev-parse --abbrev-ref HEAD` returns in each case:

   | Checkout | stdout | exit | `git symbolic-ref -q HEAD` exit | Today |
   | --- | --- | --- | --- | --- |
   | branch | `<name>` | 0 | 0 | `Branch` |
   | detached | `HEAD` | 0 | 1 | `Detached` |
   | unborn (`git init`, no commit) | `HEAD` | 128 | 0 | `Unborn` |
   | branch ref file corrupt (`zzzz`) | `HEAD` | 128 | 128 | `Unborn` (wrong) |
   | `.git/HEAD` corrupt or unreadable | empty | 128 | 128 | `Unborn` (wrong) |
   | linked worktree, gitdir missing | empty | 128 | 128 | `Unborn` (wrong; phase 9 hides it behind `Broken`) |

3. Probe type (`checkout.rs:44-81`). Add a fourth variant: `HeadState::Unreadable`, documented as "git could not read `HEAD`: the HEAD file or the ref it names is missing or corrupt". Update the type's doc list. `branch_name()` returns `None` for it; `display_label()` returns `""` for it (the finder column shows nothing, as for `Unborn`; the Git pane is where the state is reported). Serde writes it as `"unreadable"` (`rename_all = "kebab-case"`); add it to `head_state_serde_round_trip`.
4. Probe logic.
   - `branches::get_current_branch` returns `Some` only when the command exits 0 (check `output.status.success()`). This is where the defect lives. Its other caller, `RepoInfo::get` (`crates/cargo-port/src/project/git/repo/info.rs:63`), then sees `None` instead of the literal `"HEAD"` for an unborn repository. That is the correct input for `current_branch` in `RemoteResolveContext`.
   - `resolve_head_state` becomes:
     - `Some(name)` other than `GIT_HEAD` → `HeadState::Branch(name)` (one git call on the common path, as today).
     - `Some(GIT_HEAD)` → `git rev-parse --verify -q --short=8 HEAD` (`GIT_REV_PARSE_COMMAND`, `GIT_VERIFY_ARG`, `GIT_QUIET_SHORT_ARG`, `GIT_SHORT_HEAD_ARG`, `GIT_HEAD`). Exit 0 with non-empty stdout → `Detached { short_sha }`; anything else → `Unreadable`.
     - `None` → `git symbolic-ref -q HEAD` (`GIT_SYMBOLIC_REF_COMMAND`, `GIT_QUIET_SHORT_ARG`, `GIT_HEAD`). Exit 0 → `Unborn`; any other exit or a spawn error → `Unreadable`. Put this in a named function returning `HeadState` (for example `unborn_or_unreadable(repo_root)`), not a `bool`.
   - Update the `resolve_head_state` doc comment to this table. `CheckoutInfo::get` (`checkout.rs:150-187`) is otherwise unchanged. `BackgroundMsg::CheckoutInfo` carries the new variant without change.
5. Pane type (`crates/cargo-port/src/tui/panes/pane_data/git_data.rs:107-131`). Replace `Readable { head: HeadState, relation: Option<HeadRelation> }` with one flat enum whose variants are exactly the states the Git pane draws:

   ```rust
   /// The checkout as the Git pane draws it. `Broken` (a linked worktree
   /// whose gitdir is missing) wins over whatever the HEAD probe returned.
   #[derive(Clone, Default)]
   pub enum GitCheckout {
       Broken,
       #[default]
       AwaitingProbe,
       UnreadableHead,
       Unborn,
       Detached { short_sha: String },
       Branch { name: String, relation: HeadRelation },
   }
   ```

   No variant holds an `Option`. Branch-without-relation and relation-on-detached/unborn/unreadable cannot be built. `GitCheckout` no longer holds a `HeadState`.
6. `HeadRelation` (`git_data.rs:34-78`). `classify` takes the branch name and returns `Self`:
   `fn classify(branch: &str, default_branch: Option<&str>, worktree_status: Option<&WorktreeStatus>) -> Self`. Delete the `let HeadState::Branch(name) = head else { return None }` guard. Rewrite the type's doc so it describes only a branch (remove "`None` for detached and unborn checkouts").
7. `GitCheckout::from_probe` (`git_data.rs:119-131`) does the mapping:
   `fn from_probe(health: WorktreeHealth, head: Option<HeadState>, default_branch: Option<&str>, worktree_status: Option<&WorktreeStatus>) -> Self`.
   `(Broken, _)` → `Broken`. `(Normal, None)` → `AwaitingProbe` (`None` is the boundary value of `ProjectList::git_info_for` before the first probe). `Unreadable` → `UnreadableHead`, `Unborn` → `Unborn`, `Detached { short_sha }` → `Detached { short_sha }`, `Branch(name)` → `Branch { relation: HeadRelation::classify(&name, …), name }`. In `build_git_detail_fields` (`git_data.rs:342-417`) delete `head_relation` (350-356) and pass `local_main_branch.as_deref()` and `app.project_list.worktree_status_for(abs_path)` to `from_probe` (396-402). `current_branch` (378-381) still reads `HeadState::branch_name` from the probed head.
8. Callers that match the old shape. Every match is exhaustive; no `_` arm:
   - `git_fields_from_data` (`git_data.rs:244-252`): `WorktreeError` for `Broken`; `Head` for `UnreadableHead | Unborn | Detached { .. } | Branch { .. }`; neither for `AwaitingProbe`.
   - `DetailField::git_value` `Head` arm (`crates/cargo-port/src/tui/panes/pane_data/detail_field.rs:155-171`): `UnreadableHead` → `"HEAD unreadable"`, `Unborn` → `"unborn"`, `Detached` → `"detached @ {short_sha}"`, `Branch` → `"{name} · {relation.label()}"`; `Broken | AwaitingProbe` → `""`. Drop the `HeadState` import (line 9).
   - `git_field_copy_value` (`crates/cargo-port/src/tui/panes/pane_data/copy_data.rs:107-123`): `Branch` → name, `Detached` → short SHA, the rest → `""`. Drop the `HeadState` import (line 10).
   - `git_panel_title` (`crates/cargo-port/src/tui/panes/git/render.rs:1030-1047`): `Branch { name, .. }` with a non-empty name → ` Git - {name} `, `Detached` → ` Git - detached @ {short_sha} `, the rest → `pane_title("Git", …)`. Drop the `HeadState` import (line 33).
   - `build_field_value` (`render.rs:671-678`): no ` ({GIT_LOCAL} local)` suffix after `HEAD unreadable`. The suffix still follows unborn, detached and branch values.
   - Branch-row style in `base_value_style` (`render.rs:734-753`): `DetailField::Head` with `GitCheckout::UnreadableHead` takes `Style::default().fg(error_color())`, the same as the unavailable rate-limit rows. The other Branch values keep the default style.
   - `crates/cargo-port/src/tui/app/mod.rs` test fixtures `make_app_with_git_tabbable` (2916-2919) and `rebound_next_pane_uses_framework_filtered_tab_cycle` (4344-4347): `GitCheckout::Branch { name: "main".to_string(), relation: HeadRelation::Default }`. Line 1277 (`!matches!(…, AwaitingProbe)`) stays as it is: an unreadable HEAD keeps the Git pane tabbable.
   - No change needed (they only call `branch_name()` / `display_label()`): `crates/cargo-port/src/tui/project_list/list.rs:684,1676`, `crates/cargo-port/src/tui/state/ci.rs:280,293`, `crates/cargo-port/src/tui/finder/index.rs:221`, `git_data.rs:587,595`, `checkout.rs:156`.
9. Tests, each under a second. Repository fixtures use `crate::test_support::init_git_repo` (`crates/cargo-port/src/test_support.rs:43`, signing off) and `test_support::git_binary()`; a corrupt fixture is a file write after init, not more git commands.
   - `checkout.rs` `mod tests`: `resolve_head_state_reports_unreadable_head_not_unborn`. Cover four repositories: `.git/HEAD` overwritten with `garbage\n` → `Unreadable`; the branch ref named in `.git/HEAD` (`refs/heads/<name>`) overwritten with `zzzz\n` → `Unreadable`; `git init` with no commit → `Unborn`; an intact `init_git_repo` repository → `Branch(_)`. Also assert `CheckoutInfo::get(&path, None)` gives `head == HeadState::Unreadable` for the corrupt `.git/HEAD` case.
   - `git_data.rs` `git_field_tests`: update `broken_worktree_error_precedes_head_and_normal_worktree_omits_it` (664) to the new `from_probe` signature, passing `Some(HeadState::Unreadable)` for the broken case (`Broken` still wins), and to `GitCheckout::Branch { name, relation }` in place of `Readable { …, relation: None }`. Add `unreadable_head_shows_branch_row_as_head_unreadable`. `from_probe(Normal, Some(HeadState::Unreadable), …)` is `UnreadableHead`. `git_fields_from_data` lists `Head` first and no `WorktreeError`. `DetailField::Head.git_value` is `"HEAD unreadable"` and never `"unborn"`. `from_probe(Normal, Some(HeadState::Branch("main")), Some("main"), None)` is `Branch { relation: HeadRelation::Default, .. }`.
   - `app/mod.rs`, beside `selecting_worktree_with_missing_gitdir_renders_git_error` (6276), using `render_selected_git` (6216): `selecting_checkout_with_unreadable_head_renders_head_unreadable`. Steps: a tempdir package (`Cargo.toml` as in the 6276 test), then `init_git_repo`, then overwrite `.git/HEAD` with `garbage\n`. Build `make_app` with the package, select it, and apply `BackgroundMsg::CheckoutInfo` from `CheckoutInfo::get(&path, None)`. Assert: `data.checkout` matches `GitCheckout::UnreadableHead`; the rendered pane has a line holding `Branch` and `HEAD unreadable`; no line holds `unborn`; in `Appearance::Dark` and `Appearance::Light` the value cell's foreground is `tui_pane::error_color()`. Restore the theme afterwards, as `assert_git_error_colors` does.
10. Docs: one `crates/cargo-port/CHANGELOG.md` `[Unreleased]` `### Fixed` line (section at 10): "A checkout whose HEAD git cannot read shows `HEAD unreadable` in the Git pane's Branch row instead of reporting the checkout as unborn."

**Files:**
- `crates/cargo-port/src/project/git/checkout.rs` — `HeadState::Unreadable`, `resolve_head_state`, the probe test
- `crates/cargo-port/src/project/git/branches.rs` — `get_current_branch` requires exit 0
- `crates/cargo-port/src/tui/panes/pane_data/git_data.rs` — flat `GitCheckout`, `HeadRelation::classify`, `from_probe`, `build_git_detail_fields`, `git_fields_from_data`, tests
- `crates/cargo-port/src/tui/panes/pane_data/detail_field.rs` — `Head` value arm
- `crates/cargo-port/src/tui/panes/pane_data/copy_data.rs` — `Head` copy arm
- `crates/cargo-port/src/tui/panes/git/render.rs` — `git_panel_title`, `build_field_value`, Branch-row style
- `crates/cargo-port/src/tui/app/mod.rs` — two fixtures and the item 9 pane test (shared with phase 12; claim after phase 12)
- `crates/cargo-port/CHANGELOG.md` — the Fixed line (shared with phase 12; claim after phase 12)

**Workers:** 1 writer. The new `HeadState` variant and the new `GitCheckout` shape are each read by every other file in the list, and cargo-port has no `tests/` lane, so a second worker would have no file of its own.
- `probe` (opens as `impl`): every file above, lint, and the package test. Start with the probe and pane files. Take `app/mod.rs` and `CHANGELOG.md` after phase 12 releases them.

**Constraints from prior phases:** Phase 9 added `GitCheckout` with `Broken` taking precedence over any probe result, the `DetailField::WorktreeError` row (first, `on_error_color()` on `error_color()`), and `selecting_worktree_with_missing_gitdir_renders_git_error`. That test must still pass: a broken worktree's probe now returns `Unreadable`, and the pane still shows `Broken`. Phase 12 edits other tests in `app/mod.rs` and adds its own CHANGELOG line; keep both.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-port` green, with the three item 9 tests passing. Shots: the unit director's dark and light shots of the Git pane for a checkout whose `.git/HEAD` is corrupt show `Branch  HEAD unreadable` in the error color.

Open:
- Same root cause, outside this phase: `git_output_logged` returns `Ok` for a non-zero exit, so `remote_ref_exists` (`branches.rs:127-139`) and `local_branch_exists` (`branches.rs:155-167`) treat `.is_ok()` as "ref exists" and always return true. `resolve_local_main_branch` therefore always returns the configured `main_branch`. Also, `get_git_status` (`checkout.rs:404-416`) reads a failed `git status` as `Clean`, so an unreadable checkout's Status row shows `clean`. Recommendation: a separate phase that gives `git_output_logged` a typed result that carries the exit status, and moves every caller to it.

### Phase 14 — cargo-port shows every git read that fails as unreadable, never as clean, present or absent  · status: todo

**Depends on:** phase 13 (it adds `HeadState::Unreadable` and `unborn_or_unreadable`, and edits `checkout.rs`, `branches.rs`, `git_data.rs`, `detail_field.rs`, `copy_data.rs`, `git/render.rs`, `tui/app/mod.rs` and `CHANGELOG.md`, all of which this phase edits too). Also shares `crates/cargo-port/src/tui/columns/project.rs`, `tui/app/mod.rs` and `CHANGELOG.md` with phase 12, which phase 13 already follows. Nothing is shared with phases 10 or 11 (`crates/cargo-berth/`).

#### Work Order

**Goal:** A git command that cargo-port runs and that fails (spawn error, signal, or an exit status the command does not document as an answer) reaches the UI as an `Unreadable` state: the Git pane row it feeds reads `unreadable` in the error color, and the project list marks it `⛔`. No failed git is read as a ref that exists, a `clean` status, a missing value, or "no remotes". Settled by the showrunner, 2026-10-17: the list marks it `⛔` (the git column already uses wide emoji, which take no color, and an existing glyph would claim a state git never reported); CI status is hidden when a branch's upstream or the default branch cannot be read, because CI reported for a branch git never confirmed is a wrong result; `for-each-ref` replaces `@{upstream}`.

**Spec:**
1. Source: the showrunner, 2026-10-17: "Fix it at the source: git_output_logged reports a non-zero exit as a failure (a typed error, not Ok), then walk every caller so none reads a failed git as true or Clean. Add a test per caller class that a failing git surfaces as an error state."
2. The defect. `command::git_output_logged` (`crates/cargo-port/src/project/git/command.rs:25-46`) returns `Ok(Output)` for any exit status. So `branches::remote_ref_exists` (`branches.rs:127-139`) and `branches::local_branch_exists` (`branches.rs:155-167`) answer `true` for every ref (`.is_ok()`), and `resolve_local_main_branch` always returns the configured `main_branch`. `checkout::get_git_status` (`checkout.rs:378-446`) spawns `git status` through `command::git_command` and maps a spawn error to `GitStatus::Clean`. A non-zero exit leaves stdout empty, which also reads as `Clean`. Four call sites bypass `git_output_logged` and spawn `command::git_command` directly: `bisect_vars` (`checkout.rs:242`), the check-ignore and status legs of `get_git_status` (`checkout.rs:382`, `404`), and `ls_tree_submodule_commits` (`submodule.rs:152`). Exit statuses measured on natedev 2026-10-17 (git 2.x):

   | Command | intact, answer yes | intact, answer no | unborn repo | `.git/HEAD` = `garbage` |
   | --- | --- | --- | --- | --- |
   | `show-ref --verify --quiet <ref>` | 0 | 1 | 1 | 128 |
   | `check-ignore -q -- <path>` | 0 | 1 | 1 | 128 |
   | `symbolic-ref -q refs/remotes/<r>/HEAD` | 0 | 1 | 1 | 128 |
   | `rev-parse -q --verify HEAD` | 0 | — | 1 | 128 |
   | `config --get-regexp <pattern>` | 0 | 1 | 1 | 1 |
   | `rev-parse --abbrev-ref --symbolic-full-name @{upstream}` | 0 | 128 (no upstream) | 128 | 128 |
   | `for-each-ref --format=%(upstream:short) refs/heads/<b>` | 0 | 0, empty line | 0 | 128 |
   | `status`, `remote`, `for-each-ref`, `rev-list --count` | 0 | — | 0 / 128 for `log`, `ls-tree HEAD`, `rev-parse HEAD` | 128 |

   `@{upstream}` exits 128 both for "no upstream" and for a broken repository, so item 6 replaces it with `for-each-ref`, which answers "no upstream" with exit 0.
3. **The runner (`command.rs`).** Every git spawn in `crates/cargo-port/src/project/git/` goes through one of two functions. `git_command` becomes private to `command.rs`, so no caller can read a raw `Output`.

   ```rust
   /// Standard output of a git command that exited 0.
   pub(super) struct GitStdout(String);
   // trimmed(&self) -> &str, lines(&self) -> impl Iterator<Item = &str>

   /// Why git gave no answer. The runner logs it once at `warn` with `op`.
   #[derive(Debug)]
   pub(crate) struct GitReadError { op: String, cause: GitReadCause }

   #[derive(Debug)]
   pub(crate) enum GitReadCause {
       /// The git process could not be started.
       NotStarted(io::ErrorKind),
       /// git exited with a status its caller does not read as an answer.
       Exited { code: i32, stderr: String },   // first stderr line
       /// git was killed by a signal.
       Signalled,
       /// git exited 0, but its output did not parse (rev-list counts, bisect vars).
       Unparsable,
   }

   /// A command whose documented exit 1 means "no" (item 2 table, rows 1-5).
   pub(super) enum GitAnswer { Yes(GitStdout), No }

   pub(super) fn git_output_logged(repo_root: &Path, op: &str, args: &[&str]) -> Result<GitStdout, GitReadError>; // Ok only on exit 0
   pub(super) fn git_answer_logged(repo_root: &Path, op: &str, args: &[&str]) -> Result<GitAnswer, GitReadError>;  // 0 → Yes, 1 → No, else Err
   ```

   The args become `&[&str]` (not `[&str; N]`), so `bisect_vars`'s runtime-length list fits. The `git_info_get_call` trace line stays as it is. `GitReadError` implements `Display` and `std::error::Error`.
4. **Result types.** Each replaces a bool, a default or an `Option` whose `None` meant both "absent" and "failed". None holds an `Option`. All derive what the type they replace derives (`Clone`, `Debug`, `PartialEq`, `Eq`, `Serialize` where the old field had it).
   - `GitStatus` (`checkout.rs:294`) gains `Unreadable`: label `"unreadable"`, icon `GIT_STATUS_UNREADABLE` (new in `crates/cargo-port/src/constants.rs`, `"⛔"`).
   - `CommitDate { At(String), NoCommits, Unreadable }` in `checkout.rs`: `CheckoutInfo.last_commit`, and the probed value of `RepoInfo.first_commit`.
   - `FirstCommit { AwaitingProbe, Probed(CommitDate) }` (default `AwaitingProbe`) in `repo/info.rs`: `RepoInfo.first_commit`, whose `None` meant "not fetched yet".
   - `TrackedRef { Ref(String), Untracked, Unreadable }` in `checkout.rs`: `CheckoutInfo.primary_tracked_ref` and `RemoteInfo.tracked_ref` (`repo/remote.rs:47`). `CheckoutInfo::primary_tracked_ref()` keeps returning `Option<&str>`, `Some` only for `Ref`.
   - `ResolvedBranch { Named(String), NotFound, Unreadable }` in `branches.rs`: `RepoInfo.default_branch` and `RepoInfo.local_main_branch`. `CheckoutInfo::get(probe_path, local_main_branch: &ResolvedBranch)`.
   - `AheadBehind { ahead: usize, behind: usize }` and `Comparison { Counted(AheadBehind), NotCompared, Unreadable }` in `checkout.rs`: `CheckoutInfo.ahead_behind_local`, `RemoteInfo.ahead_behind`, `WorktreeInfo.ahead_behind` (`git_data.rs:226`). `NotCompared` covers every case today's `None` meant: HEAD is the local main, no local main, no tracked ref, the primary worktree row.
   - `RemoteList { Listed(Vec<RemoteInfo>), Unreadable }` in `repo/info.rs`: `RepoInfo.remotes`. `RemoteList::listed(&self) -> &[RemoteInfo]` (empty for `Unreadable`) serves the loops that only iterate. `GitOrigin` (`remote.rs:21`) gains `Unreadable`; `origin_kind` returns it for `RemoteList::Unreadable`.
   - `BisectProgress` (`checkout.rs:112`) gains `Unreadable`.
   - `PinnedCommit { Sha(String), NotPinned, Unreadable }` in `submodule.rs`: `Submodule.commit` (`submodule.rs:37`) and `SubmoduleContext.pinned_commit` (`git_data.rs:187`).
5. **Caller classes.** Each call site, the function it moves to, and what a failure becomes. `Err` never falls through to the next candidate or to a default.

   | Class | Call sites | Runner | Failure becomes |
   | --- | --- | --- | --- |
   | Existence checks | `remote_ref_exists`, `local_branch_exists` (`branches.rs`) → `Result<RefPresence, GitReadError>`, `RefPresence { Present, Absent }`; `symbolic-ref -q` for `origin/HEAD` in `get_default_branch` (`branches.rs:54`, add `GIT_QUIET_SHORT_ARG`) and for `<remote>/HEAD` in `resolve_tracked_ref` (`branches.rs:91`, add `-q`); the `check-ignore -q` leg of `get_git_status` | `git_answer_logged` | `resolve_local_main_branch` and `get_default_branch` → `ResolvedBranch::Unreadable` (`No` from every candidate → `NotFound`); `resolve_tracked_ref` → `TrackedRef::Unreadable` (`Untracked` when no step finds a ref); check-ignore → `GitStatus::Unreadable` |
   | Status reads | `git status` leg of `get_git_status` | `git_output_logged` | `GitStatus::Unreadable` |
   | HEAD reads | `get_current_branch` (`branches.rs:22`), `rev-parse --verify -q --short=8 HEAD` and `unborn_or_unreadable` (phase 13, `checkout.rs`), `rev-parse HEAD` of the primary in `worktree_ahead_behind_primary` (`checkout.rs:359`) | `get_current_branch`: `git_output_logged`; `unborn_or_unreadable`: `git_answer_logged` (`Yes` → `Unborn`, `No` or `Err` → `Unreadable`); primary HEAD: `rev-parse -q --verify HEAD` through `git_answer_logged` | Phase 13's `HeadState` mapping, unchanged. Primary HEAD `No` (unborn) → `Comparison::NotCompared`; `Err` → `Comparison::Unreadable` |
   | Value reads | last commit (`checkout.rs:164`), first commit (`repo/history.rs:15`), upstream (`get_upstream_branch`, `branches.rs:35`), default branch, remote names (`remote.rs:121`), remote URL (`remote.rs:135`), pushurls (`push.rs:93`) | `git_output_logged`; pushurls `git_answer_logged` (`No` = none set) | item 6 |
   | Counts | `parse_ahead_behind` (`checkout.rs:470`) → `Result<AheadBehind, GitReadError>`, a stdout that does not parse is `GitReadCause::Unparsable`; `bisect_vars` (`checkout.rs:234`) | `git_output_logged` | `Comparison::Unreadable`; `BisectProgress::Unreadable` (today `Awaiting`) |
   | Listings | bisect refs `for-each-ref` (`checkout.rs:202`), `ls_tree_submodule_commits` (`submodule.rs:150`) | `git_output_logged` | `BisectProgress::Unreadable` when `BISECT_START` exists (today: no bisect); item 6 for pins |

6. **Value reads in detail.**
   - Last commit (`CheckoutInfo::get`): from the probed head. `Unborn` → `CommitDate::NoCommits` and `Unreadable` → `CommitDate::Unreadable`, neither spawning git. Otherwise `log -1`: a non-empty line → `At`, `Err` or empty → `Unreadable`.
   - First commit (`get_first_commit -> CommitDate`): `log --max-parents=0 --reverse HEAD`; first line → `At`. On `Err`, run `rev-parse -q --verify HEAD`: `No` → `NoCommits`, `Yes` or `Err` → `Unreadable`. `BackgroundMsg::GitFirstCommit` carries `first_commit: CommitDate`. `ScanState::pending_git_first_commit` (`tui/state/scan.rs:32`) holds `CommitDate`. `handle_git_first_commit` and the preserve step in `repo_handlers.rs:236-242` test `FirstCommit::AwaitingProbe` where they tested `is_none()`. The `found` trace field in `background_services.rs:177` becomes `matches!(.., CommitDate::At(_))`.
   - Upstream: `get_upstream_branch(repo_root, branch: &str) -> TrackedRef` runs `for-each-ref --format=%(upstream:short) refs/heads/<branch>` (new `GIT_FORMAT_UPSTREAM_SHORT_ARG` in `project/git/constants.rs`; `GIT_LOCAL_BRANCH_REF_PREFIX` exists). Empty → `Untracked`, a name → `Ref`, `Err` → `Unreadable`. Only `HeadState::Branch(name)` spawns: `Unborn` and `Detached` → `Untracked`, `Unreadable` → `Unreadable`. `RepoInfo::get` (`repo/info.rs:55`) takes its head from `checkout::resolve_head_state` instead of `get_current_branch`, so an unborn repository gives `Untracked`, not `Unreadable`. A `TrackedRef::Unreadable` upstream makes step 1 of `resolve_tracked_ref` end the chain with `Unreadable`.
   - Remote reads: `git remote`, each `remote get-url`, and `config --get-regexp` read one config file. If any fails, `RepoInfo.remotes` is `RemoteList::Unreadable`. A per-remote tracked-ref or count failure stays on that remote (`TrackedRef::Unreadable`, `Comparison::Unreadable`).
   - Submodule pins: `ls-tree HEAD` `Ok` → `Sha` for listed paths and `NotPinned` for the rest. `Err` → `rev-parse -q --verify HEAD`: `No` (unborn parent) → every entry `NotPinned`; otherwise every entry `Unreadable`. `build_submodule_context` (`git_data.rs:627`) builds the overlay for `Sha` and `Unreadable`, not for `NotPinned`.
7. **Display.** One rule for the Git pane: a row whose state is `Unreadable` is shown even where an absent value hides the row, and its value is `unreadable` in `tui_pane::error_color()`. That is the phase 13 Branch-row style (`base_value_style`, `git/render.rs`).
   - Status: `⛔ unreadable` (`GitStatus::label_with_icon`).
   - vs local, Last commit, Inception: `unreadable`. In `git_fields_from_data` (`git_data.rs`), `CommitDate::NoCommits` and `Comparison::NotCompared` hide their rows, as `None` does today.
   - Bisect: `bisecting — progress unreadable` (`formatting::format_bisect_progress`).
   - Pinned: `unreadable` (submodule pane).
   - Remotes table: `RemoteList::Unreadable` draws one field row, new `DetailField::RemotesUnreadable`, label `Remotes`, value `unreadable`, in place of the table. Per remote, `TrackedRef::Unreadable` puts `unreadable` in the tracked column and `Comparison::Unreadable` puts it in the status column (`build_remote_rows`, `git_data.rs:497`), both in the error color.
   - Worktrees table: `Comparison::Unreadable` → `unreadable` in the error color.
   - `DetailField` copy values (`copy_data.rs`) copy the text shown.

   Project list:
   - `GitStatus::Unreadable` draws the `⛔` icon in the git path column (`columns/project.rs:565`) and the project name in `error_color()` (`project_name_style` and `project_name_shimmer_style`, `columns/project.rs:653-670`).
   - `worst_git_status` (`tui/project_list/visible_rows.rs:349`) ranks `Unreadable` above `Modified`.
   - The four sync labels (`ProjectList::git_sync` and `git_main`, `tui/project_list/list.rs:648-660` and `766-780`; `git_sync_label` and `git_main_sync_label`, `tui/panes/widths.rs:283-318`) show `⛔` for `Comparison::Unreadable`. Collapse the four ahead/behind `match` copies into one formatter in `tui/panes/pane_data/formatting.rs` beside `format_ahead_behind`, which itself takes `Comparison`.
8. **Decisions that read these states.** Each treats `Unreadable` as unknown, never as the answer it used to default to:
   - `CheckoutInfo::primary_remote` (`checkout.rs:125`) finds a remote only for `TrackedRef::Ref`.
   - `ProjectList::unpublished_ci_branch_name` (`list.rs:678-688`) and the matching test in `tui/state/ci.rs:276-281`: a branch is unpublished only for upstream `TrackedRef::Untracked` with default branch `Named(other)` or `NotFound`. With either value `Unreadable`, CI status is suppressed (`None`), because a parent-repo CI status is not known to apply to a checkout git cannot read.
   - `SyncResolution::Resolved` (`tui/project_list_state.rs:19`, read in `repo_handlers.rs`) carries `Comparison`. The sync-change tracker treats `Unreadable` as it treats `Unresolved`: no baseline, no toast.
   - `GitData::is_local` (`git_data.rs`) is true only for `RemoteList::Listed` with no remotes.
   - `tui/panes/ci/data.rs:88` and `widths.rs:291-297` compare against `GitOrigin::Local` and need no new arm.
   - Each other consumer the compiler flags matches exhaustively, with no `_` arm. Known today: `scan/background.rs:388`, `watcher/refresh.rs:296,409-422` (pass `&ResolvedBranch`), `tui/panes/pane_data/detail_data.rs:304,333,919`, `tui/app/async_tasks/dispatch.rs:154,278`, `tui/app/async_tasks/background_services.rs:171-183`, `tui/app/ci.rs`, and the `CheckoutInfo` / `RepoInfo` / `RemoteInfo` literals in `tui/app/mod.rs` tests (e.g. 4764-4785, 7928-7949, 10557-10708, 11422-11789, 12449).
9. **Tests, one or more per class, each under a second.** Fixtures:
   - `crate::test_support::init_git_repo` (`crates/cargo-port/src/test_support.rs:43`) and `test_support::git_binary()`.
   - A corrupt checkout is `.git/HEAD` overwritten with `garbage\n` after init, a file write, not more git commands. Item 2 shows every class failing on it.
   - Unborn is `git init` with no commit.

   Tests:
   - Runner (`command.rs` `mod tests`): `git_output_logged_fails_on_a_non_zero_exit`. `rev-parse --verify refs/heads/nope` in an intact repo is `Err(GitReadError { cause: Exited { code: 128, .. }, .. })`, and `rev-parse HEAD` is `Ok`. Through `git_answer_logged`, `show-ref --verify --quiet refs/heads/nope` is `Ok(No)`, `refs/heads/<current>` is `Ok(Yes(_))`, and the corrupt repo is `Err`.
   - Existence (`branches.rs` `mod tests`): `existence_checks_tell_absent_from_unreadable`.
     - `local_branch_exists(intact, "nope")` is `Ok(Absent)`; it returned `true` before this phase.
     - `remote_ref_exists(intact, "origin", "main")` with no remote is `Ok(Absent)`.
     - Both are `Err` on the corrupt repo.
     - `resolve_tracked_ref` on the corrupt repo is `TrackedRef::Unreadable`, and on an intact repo with no remote refs is `Untracked`.
     - `get_default_branch` on the corrupt repo is `ResolvedBranch::Unreadable`, and on an intact repo with no `origin/HEAD` is `NotFound`.
   - Status (`checkout.rs` `mod tests`): `git_status_of_an_unreadable_checkout_is_unreadable_not_clean`. `get_git_status` on the corrupt repo at `.` is `Unreadable`, and at a subdirectory (the check-ignore leg) is also `Unreadable`.
   - HEAD: phase 13's `resolve_head_state_reports_unreadable_head_not_unborn` still passes. `worktree_ahead_behind_primary` with a corrupt primary is `Comparison::Unreadable`, and with an unborn primary is `NotCompared`.
   - Value reads (`checkout.rs` and `repo/history.rs` / `repo/info.rs` tests): `value_reads_of_an_unreadable_checkout_are_unreadable`.
     - `CheckoutInfo::get` on the corrupt repo: `status`, `last_commit`, `primary_tracked_ref` and `ahead_behind_local` (with `ResolvedBranch::Named("main")`) are all `Unreadable`.
     - On the unborn repo: `last_commit == CommitDate::NoCommits` and `primary_tracked_ref == TrackedRef::Untracked`.
     - `get_first_commit` is `Unreadable` on the corrupt repo and `NoCommits` on the unborn one.
     - `RepoInfo::get` on the corrupt repo: `remotes` is `RemoteList::Unreadable`, `origin_kind() == GitOrigin::Unreadable`, and `local_main_branch` is `ResolvedBranch::Unreadable`.
   - Counts and listings: `parse_ahead_behind` against `HEAD...nope` is `Err`. With `.git/BISECT_START` written into the corrupt repo, `bisect_progress` is `Some(BisectProgress::Unreadable)`. `ls_tree_submodule_commits` on the corrupt repo gives `Unreadable` pins, and on an unborn parent gives `NotPinned`.
   - Pane (`tui/app/mod.rs`, beside phase 13's `selecting_checkout_with_unreadable_head_renders_head_unreadable`, using `render_selected_git`): `selecting_an_unreadable_checkout_renders_unreadable_rows`.
     - Corrupt package checkout; apply `BackgroundMsg::CheckoutInfo` from `CheckoutInfo::get`, and a `RepoInfo` from `RepoInfo::get`.
     - The rendered Git pane has a `Status` line holding `unreadable` and no line holding `clean`; a `Last commit` line holding `unreadable`; a `Remotes` line holding `unreadable`.
     - In `Appearance::Dark` and `Appearance::Light` those value cells' foreground is `tui_pane::error_color()`. Restore the theme afterwards, as `assert_git_error_colors` does.
     - The project list row draws `⛔` in the git path column.
   - Existing tests whose fixtures build `CheckoutInfo`, `RepoInfo`, `RemoteInfo` or `Submodule` literals take the new types: `Option<String>` → `CommitDate::At` / `TrackedRef::Ref` / `ResolvedBranch::Named`, `None` → the absent variant. Their assertions do not change.
10. Docs: `crates/cargo-port/CHANGELOG.md` `[Unreleased]` `### Fixed`, two lines:
    - "A git command that fails is shown as `unreadable` in the Git pane and as `⛔` in the project list, instead of reading as a clean status, an existing branch or remote ref, or a missing value."
    - "The vs-local comparison no longer assumes the configured main branch exists; it is hidden when no configured main branch exists locally."

**Files:**
- `crates/cargo-port/src/project/git/command.rs` — `GitStdout`, `GitReadError`, `GitReadCause`, `GitAnswer`, `git_output_logged`, `git_answer_logged`, private `git_command`, runner tests
- `crates/cargo-port/src/project/git/constants.rs` — `GIT_FORMAT_UPSTREAM_SHORT_ARG`
- `crates/cargo-port/src/project/git/branches.rs` — `RefPresence`, `ResolvedBranch`, existence checks, `get_upstream_branch`, `get_default_branch`, `resolve_tracked_ref`, `resolve_local_main_branch`, tests
- `crates/cargo-port/src/project/git/checkout.rs` — `GitStatus::Unreadable`, `CommitDate`, `TrackedRef`, `AheadBehind`, `Comparison`, `BisectProgress::Unreadable`, `get_git_status`, `CheckoutInfo::get`, `parse_ahead_behind`, bisect reads, `worktree_ahead_behind_primary`, tests
- `crates/cargo-port/src/project/git/submodule.rs` — `PinnedCommit`, `ls_tree_submodule_commits`, test
- `crates/cargo-port/src/project/git/repo/history.rs` — `get_first_commit -> CommitDate`, test
- `crates/cargo-port/src/project/git/repo/remote.rs` — `RemoteInfo.tracked_ref` / `ahead_behind` types, `GitOrigin::Unreadable`, `list_remote_names`, `remote_url_info`
- `crates/cargo-port/src/project/git/repo/push.rs` — `list_remote_pushurls` through `git_answer_logged`
- `crates/cargo-port/src/project/git/repo/info.rs` — `RemoteList`, `FirstCommit`, `RepoInfo::get`, `origin_kind`, test
- `crates/cargo-port/src/project/git/repo/mod.rs`, `crates/cargo-port/src/project/git/mod.rs`, `crates/cargo-port/src/project/mod.rs` — re-exports of the new types
- `crates/cargo-port/src/constants.rs` — `GIT_STATUS_UNREADABLE`
- `crates/cargo-port/src/scan/background.rs` — `CheckoutInfo::get` call
- `crates/cargo-port/src/watcher/refresh.rs` — `CheckoutInfo::get` calls with `&ResolvedBranch`
- `crates/cargo-port/src/tui/state/scan.rs` — `pending_git_first_commit` holds `CommitDate`
- `crates/cargo-port/src/tui/state/ci.rs` — unpublished-branch check
- `crates/cargo-port/src/tui/app/async_tasks/background_services.rs` — first-commit fetch
- `crates/cargo-port/src/tui/app/async_tasks/dispatch.rs` — `GitFirstCommit` and submodule commit handling
- `crates/cargo-port/src/tui/app/async_tasks/repo_handlers.rs` — first-commit preserve step, sync tracker
- `crates/cargo-port/src/tui/project_list_state.rs` — `SyncResolution::Resolved(Comparison)`
- `crates/cargo-port/src/tui/app/ci.rs` — primary remote / URL reads
- `crates/cargo-port/src/tui/project_list/list.rs` — `git_sync`, `git_main`, `primary_ahead_behind_for`, `primary_sync_resolution`, `fetch_url_for`, `unpublished_ci_branch_name`
- `crates/cargo-port/src/tui/project_list/visible_rows.rs` — `worst_git_status`
- `crates/cargo-port/src/tui/columns/project.rs` — `⛔` git path icon, name styles (shared with phase 12)
- `crates/cargo-port/src/tui/panes/widths.rs` — sync labels
- `crates/cargo-port/src/tui/panes/ci/data.rs` — `RemoteList` iteration
- `crates/cargo-port/src/tui/panes/pane_data/git_data.rs` — `GitData` fields, `git_fields_from_data`, `build_git_detail_fields`, `build_remote_rows`, worktree rows, `build_submodule_context`, `is_local`
- `crates/cargo-port/src/tui/panes/pane_data/detail_field.rs` — `RemotesUnreadable`, unreadable values
- `crates/cargo-port/src/tui/panes/pane_data/detail_data.rs` — last commit and submodule version
- `crates/cargo-port/src/tui/panes/pane_data/copy_data.rs` — copy values
- `crates/cargo-port/src/tui/panes/pane_data/formatting.rs` — one ahead/behind formatter over `Comparison`, bisect text
- `crates/cargo-port/src/tui/panes/git/render.rs` — error color for unreadable values, remotes and worktree tables
- `crates/cargo-port/src/tui/app/mod.rs` — fixture literals and the item 9 pane test (shared with phases 12 and 13)
- `crates/cargo-port/CHANGELOG.md` — two Fixed lines (shared with phases 12 and 13)

**Workers:** 1 writer. The caller classes do not split into disjoint file sets: status, last commit, counts, bisect and the HEAD reads all live in `checkout.rs`, and existence checks and value reads both live in `branches.rs`. Every `tui/` file consumes the item 4 types and does not compile until they exist. cargo-port has no `tests/` lane, so a second worker would hold no file of its own.
- `probe` (opens as `impl`): every file above, lint, and the package test. Start with `command.rs`, then the `project/git/` files class by class, then the `tui/` consumers the compiler names.

**Constraints from prior phases:**
- Phase 9: `GitCheckout::Broken` wins over any probe result, and `selecting_worktree_with_missing_gitdir_renders_git_error` must still pass.
- Phase 13: `HeadState::Unreadable`, the flat `GitCheckout` (no `Option<HeadRelation>`), `unborn_or_unreadable`, and the Branch row `HEAD unreadable` in `error_color()`. This phase changes only the return types under them; their mapping and tests stay.
- Phase 12: `on_error_color()` text on `error_color()` for broken-worktree rows in `columns/project.rs` and `tree_rows.rs`. Keep it; the `⛔` icon and error-colored name apply only to rows that are not `WorktreeHealth::Broken`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-port` green, with the item 9 tests passing: `git_output_logged_fails_on_a_non_zero_exit`, `existence_checks_tell_absent_from_unreadable`, `git_status_of_an_unreadable_checkout_is_unreadable_not_clean`, phase 13's HEAD test, `value_reads_of_an_unreadable_checkout_are_unreadable`, the counts and listings tests, and `selecting_an_unreadable_checkout_renders_unreadable_rows`. `grep -n "git_command(" crates/cargo-port/src/project/git/` finds calls only inside `command.rs`. Shots: the unit director's dark and light shots of the Git pane for a checkout whose `.git/HEAD` is corrupt show Status, Last commit and Remotes as `unreadable` in the error color, and the project list row shows `⛔`.

### Phase 15 — cargo-port's project list never cuts off a broken worktree's ` [broken]` suffix  · status: todo

**Depends on:** phase 12 (shares `tree_rows.rs`; its suffix style stays). Shared with phase 13: `crates/cargo-port/src/tui/app/mod.rs` and `crates/cargo-port/CHANGELOG.md`; claim those two paths `--after` the reservation that holds them (phase 12's, then phase 13's). Phase 14 also lists `crates/cargo-port/src/tui/panes/widths.rs` (sync labels); claim it `--after` phase 14's reservation if phase 14 holds it. The constants, widths and `tree_rows.rs` edits start at once. Nothing is shared with phases 10 or 11 (`crates/cargo-berth/`).

#### Work Order

**Goal:** A broken linked worktree's row in the project list shows its disk size and the whole ` [broken]` suffix at any terminal width the list fits in: the Disk column's fitted width counts the suffix, so at 160 columns the row reads ` 304 B [broken]`, not ` 304 B [b`.

**Spec:**
1. Source: the showrunner, 2026-10-17: "Count the suffix in the disk column's width so ` [broken]` is never cut, and checkpoint with a shot at 160 columns showing the full suffix."
2. The defect. The project list pane is exactly as wide as its fitted columns: `render.rs:224-226` sets the left width to `cached_fit_widths.total_width() + BLOCK_BORDER_WIDTH`, and Disk (`COL_DISK = 7`, `columns/constants.rs:3`) is the last column. The fit pass (`crates/cargo-port/src/tui/panes/widths.rs`, `compute_project_list_widths` at 60, called from `App::ensure_fit_widths_cached` in `crates/cargo-port/src/tui/app/navigation/cache.rs:21`) observes Disk with the size text alone: `widths.observe(COL_DISK, dw(&formatted_disk(..)))` at widths.rs 94, 125, 165, 189, 208 and 225. `row_to_line` (`crates/cargo-port/src/tui/columns/project.rs:421-431`) draws a suffixed cell as the size padded to `col_width - suffix_width` (saturating, never truncated) followed by the suffix, so a ` [broken]` row is 9 columns wider than its column and the list clips it at the pane edge. Measured by the showrunner at 160 columns: ` 304 B [b` (Disk column 8 wide, from a `36.3 GiB` row).
3. One suffix constant. Add to `crates/cargo-port/src/tui/panes/constants.rs`, beside the tree prefix constants (which carry the same "single source of truth for width calc and render" rule):
   ```rust
   /// Disk-cell suffix on a `WorktreeHealth::Broken` row; measured by
   /// `panes::widths` and drawn by `panes::project_list`.
   pub(super) const BROKEN_WORKTREE_DISK_SUFFIX: &str = " [broken]";
   ```
   `disk_suffix_for_state` (`crates/cargo-port/src/tui/panes/project_list/tree_rows.rs:653-673`) returns `Some(BROKEN_WORKTREE_DISK_SUFFIX)` in its `Broken` arm in place of the literal at 667; import it as the other `crate::tui::panes::constants::` items at 30-39 are. Its style (phase 12's `on_error_color()` on `error_color()`) and the deleted ` [x]` arm stay as they are.
4. Width rule (`widths.rs`). Add one helper and route the three measures whose rows draw `disk_suffix_for_state` through it:
   ```rust
   fn observe_disk_width(widths: &mut ProjectListWidths, bytes: Option<u64>, health: WorktreeHealth) {
       let suffix_width = match health {
           WorktreeHealth::Normal => 0,
           WorktreeHealth::Broken => columns::display_width(BROKEN_WORKTREE_DISK_SUFFIX),
       };
       widths.observe(COL_DISK, columns::display_width(&formatted_disk(bytes)) + suffix_width);
   }
   ```
   The `match` is exhaustive, no `_` arm, so a new `WorktreeHealth` variant (`crates/cargo-port/src/project/info.rs:23`) must state its suffix width. Each measure pairs with the row that draws it:
   - widths.rs 94, root row: `observe_disk_width(widths, item.disk_usage_bytes(), item.worktree_health())` — drawn at tree_rows.rs 415-417 with `item.worktree_health()`.
   - widths.rs 125, `observe_path_only_entry_fit_widths` (submodules): `entry.info().disk_usage_bytes`, `entry.info().worktree_health` — drawn by `render_path_only_entry` at tree_rows.rs 951-952 with the same field.
   - widths.rs 208 (`observe_workspace_worktree_entry_fit_widths`) and 225 (`observe_package_worktree_entry_fit_widths`), worktree entries: `ws.worktree_health()` / `pkg.worktree_health()` (`ProjectFields`) — drawn at tree_rows.rs 587-589 through `worktree_health_for_entry`.
   - widths.rs 165 (members) and 189 (vendored) stay as they are: those rows (tree_rows.rs ~504) never draw the broken suffix, and counting it there would widen the column for no drawn text.
   Import `WorktreeHealth` from `crate::project` and the constant from `super::constants`. Update the module doc (widths.rs 1-8) so it says the Disk column's width is the size text plus the row's suffix.
5. Nothing else changes: `row_to_line`, the suffix text, its style, and which rows count as broken stay as they are. A row that is not broken keeps its current Disk width.
6. Test, under a second, in `crates/cargo-port/src/tui/app/mod.rs` `mod interaction`, beside phase 12's `broken_worktree_project_row_draws_on_error_text_on_error_red` (~6360): `broken_worktree_disk_suffix_is_drawn_whole_at_160_columns`. It drives the real frame, not `render_tree_buffer`:
   - Fixture as phase 12's test: a primary and a linked `make_package_worktree` in one `WorktreeGroup`, the linked one with `rust.project_info.worktree_health = WorktreeHealth::Broken` and `rust.project_info.disk_usage_bytes = Some(304)`, the primary with `rust.project_info.disk_usage_bytes = Some(39_000_000_000)` (formats as `36.3 GiB`, the showrunner's case); `app.project_list.expanded.insert(ExpandKey::Node(0))`.
   - For `Appearance::Dark` and `Appearance::Light` (set as phase 12's test does): run the event loop's cache order (`crates/cargo-port/src/tui/terminal/event_loop.rs:104-107`) — `ensure_visible_rows_cached`, `ensure_disk_cache`, `ensure_fit_widths_cached`, `ensure_detail_cached` — then draw `render::ui` on `TestBackend::new(160, 40)`.
   - Assert: the buffer row holding `broken-linked` also holds `304 B [broken]`; each of the 9 cells of ` [broken]` has `fg == tui_pane::on_error_color()` and `bg == tui_pane::error_color()`.
   - Restore the theme afterwards, as phase 12's test does.
   - Run it against the code before items 3-4 first: it fails there on the cut suffix.
7. Docs: one `crates/cargo-port/CHANGELOG.md` `[Unreleased]` `### Fixed` line: "A broken worktree's `[broken]` suffix in the project list is no longer cut off; the Disk column's width now counts it."

**Files:**
- `crates/cargo-port/src/tui/panes/constants.rs` — `BROKEN_WORKTREE_DISK_SUFFIX`
- `crates/cargo-port/src/tui/panes/widths.rs` — `observe_disk_width`, the three suffixed Disk measures, module doc (also in phase 14's Files)
- `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs` — `disk_suffix_for_state` draws the constant (shared with phase 12)
- `crates/cargo-port/src/tui/app/mod.rs` — the item 6 test (shared with phases 12 and 13)
- `crates/cargo-port/CHANGELOG.md` — the Fixed line (shared with phases 12 and 13)

**Workers:** 1 writer. The change is one constant read by two files and one test; cargo-port has no `tests/` lane, so a second worker would have no file of its own.
- `impl` — every file above, lint, and the package test. Start with `constants.rs`, `widths.rs` and `tree_rows.rs`; take `app/mod.rs` and `CHANGELOG.md` when phases 12 and 13 release them.

**Constraints from prior phases:** Phase 12 styles the ` [broken]` suffix and the broken row's spans `on_error_color()` on `error_color()` (`tree_rows.rs` `disk_suffix_for_state`, `columns/project.rs` `row_to_line`); keep that style. Phase 12's `broken_worktree_project_row_draws_on_error_text_on_error_red` reads ` [broken]` from `render_tree_buffer`, a buffer exactly `widths.total_width()` wide, so it reads the whole suffix only once item 4's width rule is in; keep it. Phases 12 and 13 each add a CHANGELOG `Fixed` line; keep both.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-port` green, with `broken_worktree_disk_suffix_is_drawn_whole_at_160_columns` passing. Shots: the unit director's dark and light shots of the project list at 160 columns with a broken worktree row show the full ` [broken]` suffix, white on the error red.

Open:
- Same class, outside the showrunner's request: a deleted row draws `0.0` plus ` [x]` (7 columns, tree_rows.rs `disk_suffix_for_state` and ~504) while its Disk measure counts only the size text, so a list whose widest size is under 7 columns cuts ` [x]` the same way. Recommendation: a follow-up that measures the deleted suffix through the same helper, with `ProjectList::is_deleted` passed to the fit pass.
