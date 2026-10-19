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

**Depends on:** phase 11 (`crates/cargo-berth/`)

#### As-built

- Mac failures fixed at their cause. `inherited_git_config_overlay_survives_hook_suppression` (`src/git/command.rs`) initializes its own temporary repository instead of reading the ambient checkout. The edges successor tests read a linked checkout's administration through `ResolvedLinkedAdministration::read(root)` (`tests/support/linked_checkout.rs`), which canonicalizes both the gitdir pointer and `commondir` before `locator()` strips one from the other; `administrative_directory` is built on it. Template sources and destinations are canonicalized before path substitution, so a macOS `/private/var` root never doubles to `/private/private/var`.
- Managed hook layout (`src/gate/install.rs`): the `reference-transaction` hook is one script, byte-identical across repositories (`ManagedHook::shared_script`), that sources `cargo-berth-hook-context` (mode `0o644`) beside it for `cargo_berth_policy_worktree`, the trunk reference, gate-targets path, loose trunk ref and pending-marker prefix and suffix. A missing context prints a rerun-`cargo berth init` notice and permits the transaction. `RepositoryHookContext::install` and `replace_outdated_reference_transaction_hook` write the context through the same atomic `PendingManagedHookReplacement` as the hook; ownership is matched in the script or the context. The template fingerprint hashes the shared script.
- Fixture executables: `DirectorySnapshot::restore` hard-links each managed hook (`#!/bin/sh\n# cargo-berth managed hook:`) to one immutable inode per body in `cargo-berth-test-executables-$USER` under the system temp directory, executed once under a lock to pay the first-exec cost. `DirectorySnapshot::write_shared_shell_script(path, script)` does the same for test editors (`reanchoring.rs`, `split_rebase.rs`, `board.rs`, `drift.rs`, `edges.rs`, `gate.rs`); per-copy data goes in a private file beside the script.
- Recorded templates: nine more `BeforeRun` templates in `edges_templates.rs` (`MERGE_EXTENT_MAIN`, `MERGE_EXTENT_PAIR`, `INITIAL_INTEGRATION_PAIR`, `EQUIVALENT_SUCCESSOR`, `DIFFERENT_SUCCESSOR`, `ONE_PREDECESSOR`, `TWO_PREDECESSORS`, `HISTORICAL_PROOF`, `COVER_LANDING`), 24 in all, built in the nextest setup instead of the first tests' clocks. `LedgerTemplate::instantiate_with_external_worktrees` and `IntegrationRepository::from_recorded_template` place a template's linked checkouts beside the copy; `IdentityRenewal::apply_pair` renews owners across both. `tests/gate.rs` instantiates an `INITIALIZED_REPOSITORY` template.
- Setup: `setup-recorded-templates.sh` reads `CARGO_BERTH_TEMPLATE_CACHE` (default `${CARGO_TARGET_DIR:-target}/tmp/cargo-berth-templates`); when no builder was listed for this `NEXTEST_RUN_ID`, it runs `cargo build -p cargo-berth --test recorded_templates` and executes that builder, so a cold `--test <target>` run builds every template. `cold_single_target_setup_builds_every_recorded_template` runs the wrapper against an empty cache and checks every template is complete. `.config/nextest.toml` requires nextest 0.9.138.
- Times, cold: natedev quiet 1145/1145 in 9.055 s, slowest 0.441 s (21.0 s with three tests at 1 s or more before). Mac 1146/1146 pass in 93.1 s, 354 tests at 1 s or more, SETUP 10.6 s; the cost there is process starts at suite concurrency, and fsync is a 0–6 % share. CI: 35 tests at 0.7 s or more, 13 at 1 s or more, and template SETUP 2.1 s → 37.7 s because the setup fallback `cargo build` recompiles in CI.

**Files:**
- `crates/cargo-berth/src/gate/install.rs` — shared reference-transaction script, `RepositoryHookContext`, context refresh
- `crates/cargo-berth/src/git/command.rs` — hook-suppression test on its own repository
- `crates/cargo-berth/src/git/patch.rs` — `rewrite_mapping_stops_at_onto_independently_of_other_refs` builds its history once and restores a snapshot per ref kind
- `crates/cargo-berth-test-support/src/directory_snapshot.rs` — `ImmutableShellExecutable`, `write_shared_shell_script`
- `crates/cargo-berth-test-support/src/{repository_template,integration_repository}.rs` — canonical relocation, external-worktree instantiation, prebuilt pairs
- `crates/cargo-berth/tests/recorded_templates.rs`, `tests/setup-recorded-templates.sh`, `.config/nextest.toml` — builder list, single-target setup and its test, version floor
- `crates/cargo-berth/tests/support/{edges_templates,linked_checkout,reanchoring,split_rebase}.rs` — new templates, resolved administration, shared editors
- `crates/cargo-berth/tests/{board,drift,edges,gate,lifecycle}.rs` — tests on templates and shared scripts

**Binds later work:** "The cargo-berth suite starts few enough processes that every test runs under a second on the Mac" owns the Mac every-test-under-1 s goal and, first, the CI SETUP regression from the setup fallback `cargo build`. A hook change keeps the shared script plus private `cargo-berth-hook-context` layout and replaces both atomically; `shared_reference_body_dispatches_with_each_private_repository_context` and `shared_script_replacement_keeps_its_other_copy_and_mutable_files_private` stay.

**Gotchas:**
- Fixture hooks and shared editors are hard links to one inode: a test that edits an installed hook unlinks it first (`write_private_hook` in `gate.rs`, `remove_file` in `drift.rs`), or it rewrites every copy.
- Deleting `target/tmp/cargo-berth-templates` leaves the executable cache in the system temp directory warm; a cold timing run clears both, uses `--no-cache` (`verify.sh test` otherwise replays an earlier result) and samples load during the run.

**Ruled out:** thread-count changes, raised limits, `#[ignore]`, macOS-only skips or a suite `TMPDIR` as timing or Mac fixes; per-test cuts on a saturated Mac, which only move time between tests.

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

### Phase 18 — The cargo-berth suite starts few enough processes that every test runs under a second on the Mac  · status: done

#### As-built

- `git::RepositorySnapshot` (`src/git/snapshot.rs`) holds one discovered worktree context and one persistent `git cat-file --batch` object reader per berth operation. Board evaluation, reconciliation, rewrite preflight and gate preparation borrow it; a second snapshot or reader inside one operation has no constructor path.
- `for-each-ref` reads every branch's upstream into one map, so an optional upstream (`@{…}`) never enters the batch object reader. Process spawns are serialized, so the persistent reader child inherits no open pipe end of another spawn (the cause of the Mac template SETUP stall).
- `tests/process_start_budget.rs` audits process starts per test by origin and pins the suite total in `tests/process_start_budget.json` at 46,393 (product Git 26,112; fixture Git 6,390; berth 3,174; hook berth 1,335). It replaces the Python auditor.
- The recorded-template SETUP fails a real stall after 120 s and names each unfinished template (`require_template_progress`).
- `cargo-berth check --help` says the check acquires the paths for this session when they are clear, and that `cargo-berth board` reads holders without acquiring.
- The full suite passes on the Mac (1175 tests, 105.8 s cold); 389 tests still take 1 s or more there.

**Files:**
- `crates/cargo-berth/src/git/snapshot.rs` — the per-operation snapshot and object reader
- `crates/cargo-berth/tests/process_start_budget.rs`, `crates/cargo-berth/tests/process_start_budget.json` — the start auditor and its pin
- `crates/cargo-berth/tests/recorded_templates.rs`, `crates/cargo-berth/tests/setup-recorded-templates.sh` — template SETUP and its stall bound

**Binds later work:** Every Git read in an operation goes through its one `RepositorySnapshot`. The start pin may only go down.

**Gotchas:** Mac wall time follows machine load more than start count: between two cold runs the starts fell while the total rose 96.7 → 105.8 s, with every binary slower at load 23 on 12 cores. Record the load with every Mac timing.

**Ruled out:** Holding this phase open for the Mac per-test goal; that goal is its own phase, built after the route estimates.

### Phase 19 — A cargo-berth write holds the ledger lock only for its own check and append  · status: done

#### As-built

- Session-index changes (`PreparedOperationSessionChange`, `PreparedSessionMappingPublication`, `PreparedCurrentSessionRemoval`), reconciliation evidence (`GenerationBoundReconciliation`, `ReconciliationObservationPass` with `RepositorySnapshot::fresh_observation`) and gate preparation run before `MutationLock::acquire`.
- Under the lock, only the tail advance, the check of the replay generation or `SessionMappingVersion`, the append and the publish run. Each prepared result carries its generation by type, so a result with no generation cannot reach an append.
- When later events make a prepared result stale, the lock is released and the result is prepared again: at most `MUTATION_PREPARATION_ATTEMPTS` (8) times, with `MUTATION_RETRY_WINDOW_BASE_MILLISECONDS` (2) backoff, then a clear error. A stale result is never appended.
- `PreparedRetentionRefTransaction` (`git/refs.rs`) replaces `update_reservation_retention_refs`. It starts `git update-ref --stdin` and gets the `start` acknowledgement before the lock, and sends the ref updates and `commit` after the journal append. `Drop` kills and reaps the child.
- `JournalReplaySource(device, inode)` in `advance_read_only_tail` makes a compaction swap cause a full replay, never a tail read.
- Busy reconcile hold on a 133 MB scratch journal: p95 9105.8 ms → 52.4 ms, max 68.5 ms. The quiet-load proof is not established; the quiet re-measure belongs to "Every cargo-berth test runs under a second on the Mac".

**Files:**
- `crates/cargo-berth/src/ledger/handle.rs` — prepare, check and append entry points and the bounded retry
- `crates/cargo-berth/src/ledger/journal.rs` — read-only tail advance with the replay source check
- `crates/cargo-berth/src/session/mod.rs` — prepared session-index publications and removals
- `crates/cargo-berth/src/reconcile.rs` — generation-bound reconciliation and its retention commit
- `crates/cargo-berth/src/git/refs.rs`, `git/command.rs`, `git/snapshot.rs` — the prepared retention transaction and the fresh observation

**Binds later work:** Retention refs change only through `PreparedRetentionRefTransaction::{empty, prepare, prepare_reconciliation, for_committed_hook, commit}` (reconcile, committed hooks) and the explicit writes `write_reservation_retention_ref` (`verb/release.rs`, `reservation/evidence.rs`) and `delete_reservation_retention_refs` (`verb/release.rs`, `recovery.rs`). A stale Git lock now surfaces at `commit` after the append or at `start` in `prepare`. New lock paths use `MUTATION_PREPARATION_ATTEMPTS` and `MUTATION_RETRY_WINDOW_BASE_MILLISECONDS`, and carry their replay generation or `SessionMappingVersion` by type. A `JournalReplaySource` mismatch triggers a full replay.

**Gotchas:** A ref `commit` after a durable append can fail: the call returns an error and the event stays in the journal. Each nonempty retry starts another update-ref process. Slow busy holds come from journal sync (340 ms) and ordered retention-ref writes (214/134 ms), not from sync alone.

### Phase 20 — A stale git lock never makes the cargo-berth board unavailable  · status: todo

#### Work Order

**Depends on:** phase 19 (it changes `git/refs.rs` and the read and write paths this phase touches).

**Goal:** A `packed-refs.lock` that no process holds never makes a cargo-berth read fail. A read still answers the board, and a refused Git write says which lock refused it, how old it is and whether a process holds it.

**Spec:**
1. Source: the showrunner, 2026-10-19, after a 0-byte `/home/natepiano/rust/cargo-liner/.git/packed-refs.lock` stayed from 01:03:15 to 01:11. Cause: a user interrupt cancelled a PostToolUse `cargo-berth hook post-tool-use`, and its `git update-ref --stdin` retention-ref transaction died after it made the lock. While the lock stayed, every ledger read failed with "git update-ref failed … packed-refs.lock: File exists", and four style units stopped at refill with "board unavailable". The harness can stop a hook at any time, so the fix is on the read side. Approved verbatim: "(1) a read defers the retention-ref repair and still answers; (2) a refused git write reports the lock path, its age and whether a process holds it, and cargo-berth does not delete the lock."
2. **A read defers retention maintenance.** Board, check, drift and hook reconciliation prepare retention maintenance through `git::PreparedRetentionRefTransaction::{prepare_reconciliation, for_committed_hook}` and apply it through `commit` (`git/refs.rs`). Transaction startup runs before the journal append and queues no ref change; the stale `packed-refs.lock` refusal comes when the changes are sent at `commit`, after the append (`reconcile.rs` `commit_repairs` and the committed-hook action; `ledger/handle.rs` append then committed action). A Git-lock refusal there produces a typed deferred outcome: the durable journal result stays, the read completes and its output names the deferred work once. The next reconciliation rebuilds the unfinished repairs and deletions from journal state and runs them. Do not retry the journal append, and do not hide Git failures that are not lock refusals. Today `commit_repairs` returns before its remaining marker maintenance when the ref commit fails; that remaining maintenance must still run.
3. **A refused write names the lock.** When a Git ref write fails because a `*.lock` exists, the error is a typed variant that carries the lock path, its age from the file's modification time, and whether a process holds it open. Its text states all three. cargo-berth never deletes the lock. Explicit writes keep their failure: `write_reservation_retention_ref` (from `verb/release.rs` and `reservation/evidence.rs`) and `delete_reservation_retention_refs` (from `verb/release.rs` and `recovery.rs`) are not deferrable read maintenance; they fail with this enriched error.
4. **Tests** in `tests/ledger.rs`, each named for the behavior it pins, with a 0-byte `packed-refs.lock` that no process holds and a real pending retention deletion: a read whose reconciliation appends an event before the ref maintenance fails still answers, prints one deferred-maintenance notice and keeps that event exactly once; after the fixture removes its lock, the next reconciliation completes the deletion without a second event; the repair outcome and committed-hook maintenance are covered the same way, and the other marker maintenance is not abandoned; an explicit ref write that needs `packed-refs.lock` fails with the lock path, its age and "no process holds it", leaves the lock in place and keeps any event already appended; a Git failure that is not a lock refusal still fails.

**Files:**
- `crates/cargo-berth/src/git/refs.rs`, `crates/cargo-berth/src/git/error.rs`, `crates/cargo-berth/src/git/command.rs` — prepared and immediate ref-write failures and the typed lock refusal
- `crates/cargo-berth/src/reconcile.rs`, `crates/cargo-berth/src/ledger/handle.rs` — deferred maintenance after the durable append, committed-hook reconciliation included
- `crates/cargo-berth/src/output.rs`, `crates/cargo-berth/src/verb/board.rs`, `crates/cargo-berth/src/verb/check.rs`, `crates/cargo-berth/src/drift/execution.rs` — carry the deferred outcome to read output without losing the answer
- `crates/cargo-berth/tests/ledger.rs` — the stale-lock read, write and retry tests

**Workers:** 1 writer, `mac2`, owning the implementation and `tests/ledger.rs`. It built phase 19's retention-ref transaction.

**Constraints:** No deletion of any Git lock by cargo-berth. No raised lock tolerance or longer timeout. The process-start budget may only go down. Runs beside phase 21: do not edit `drift/classification.rs`, `drift/provenance.rs` or `tests/hooks.rs`.

**Constraints from prior phases:** Phase 19 prepares observations outside the ledger lock and checks replay generation and session mappings under it. Keep the bounded re-preparation and its exhaustion error; never retry a durable append because a later ref action failed. A replaced journal source triggers a full replay, with no new user-facing state. The phase 19 invalidation and compaction tests stay authoritative.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` pass, with both tests.

### Phase 21 — Merging a covered branch into its target never reports an incursion  · status: todo

#### Work Order

**Depends on:** phase 19 (it changes `drift/execution.rs` and the ledger paths the post-write check reads).

**Goal:** When a merge brings a covered branch's own commits into the cover's target, the post-write check treats it as the integration the cover exists for. It reports no incursion and tells nobody to stop.

**Spec:**
1. Source: the showrunner, 2026-10-19. It merged style hand-off 12cc3b9 (`style/main`) into `main` in `/home/natepiano/rust/cargo-liner` with `merge_checkpoint.py`. The PostToolUse hook printed "POST-WRITE INCURSION: nothing was reserved … crates/cargo-port/src/lint/reclaim.rs inside foreign reservations 01a15345-3e06-75b3-a3e3-457fb29c00a7 … stop and resolve the incursion". That reservation is the cover for `style/main` on `main` (source `cover`, purpose "Cover integration branch style/main"). The merge brought in only the covered branch's commits. The next board read showed the cover as resolved and no outstanding incursion, so only the post-write report was wrong.
2. **A covered integration is not an incursion.** In drift's foreign-path classification (`drift/classification.rs`), exempt only the committed contribution of one particular cover: the checkout is that cover's recorded integration target (`worktree/enrollment.rs`, the cover's `covered_branch` and its target are separate facts), and every commit that brings the path in is affirmatively reachable from that cover's covered branch tip. Classification today returns one decision for the whole blocker list (`classification.rs` near line 345); the decision becomes per holder, so a cover's exemption never hides another holder's incursion. Covered integration, entering work and unavailable evidence are explicit types the classification returns, never a bare `Option` or an absence read as coverage, and never a guard in the report.
3. **Other writes into a cover stay incursions.** A commit that changes a covered path and that the covered branch does not contain, a working-tree write beyond the merge, and a path another holder holds are still incursions, with today's text. Unavailable covered-branch evidence (a missing anchor gives an empty range today, `git/reachability.rs` near line 1025) stays a conservative typed outcome that reports the incursion.
4. **Tests** in `tests/hooks.rs`, each named for the behavior it pins, each through the real post-write hook before any board read can settle the cover, each checking the hook's text and the journal outcome: merging a covered branch into its target reports no incursion and leaves no outstanding incursion; a commit outside the covered branch that changes a covered path is reported; a dirty working-tree addition after the merge is reported; a merge that also enters another holder's path reports that holder; unavailable covered-branch evidence reports the incursion.

**Files:**
- `crates/cargo-berth/src/drift/classification.rs`, `crates/cargo-berth/src/drift/provenance.rs` — the per-holder covered-integration decision and the covered-branch ancestry read
- `crates/cargo-berth/tests/hooks.rs` — the post-write hook tests

**Workers:** 1 writer, `mac`, owning `drift/classification.rs`, `drift/provenance.rs` and `tests/hooks.rs`.

**Constraints:** No new Git process per hook call and none per cover: read the distinct covered-branch tips inside the existing preparation snapshot and reuse the batched ancestry read in `drift/provenance.rs` (near line 443; its target selection near lines 153–159 does not yet establish covered-branch reachability). The process-start budget may only go down. Runs beside phase 20: do not edit `drift/execution.rs`, `reconcile.rs`, `ledger/`, `git/refs.rs`, `output.rs`, `verb/` or `tests/ledger.rs`.

**Constraints from prior phases:** Phase 19 prepares observations outside the ledger lock and checks replay generation and session mappings under it; drift preparation reads the per-operation `RepositorySnapshot` before the acquire. Keep the bounded re-preparation and its exhaustion error. The phase 19 invalidation and compaction tests stay authoritative.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` pass, with both tests.

### Phase 22 — Every cargo-berth test runs under a second on the Mac  · status: todo

#### Work Order

**Depends on:** phase 19 (its lock and preparation changes move the same Git reads this phase measures), phase 20 and phase 21.

**Goal:** A cold full `cargo nextest run -p cargo-berth` on the Mac passes every test, each in under 1 s. The showrunner picks the route from measured estimates before it is built.

**Spec:**
1. Source: the showrunner, 2026-10-18, verbatim: "Then plan the Mac per-test goal as its own phase. Before you build it, send me a measured estimate for each route on the five slowest binaries (lifecycle, gate, board, parallel_phases, drift): (1) berth commands in-process in the tests, with shared fixture repositories; (2) gix for the status, worktree list and ref reads, which are the largest product Git origins. Give the starts and the Mac time for each route. Route 1 must still test the real CLI in at least one test for each command. I decide with those numbers."
2. Baseline: phase 18's cold Mac run (1175 tests, 105.775 s, 389 tests at 1 s or more, SETUP 11.8 s, load 6 → 23) and its start count 46,393 (product Git 26,112; fixture Git 6,390; berth 3,174; hook berth 1,335). First take a cold full Mac timing and process-origin measurement of the composed phase 19+20+21 checkpoint, recording its commit, load, setup cost and complete test count. Compare it with the phase 18 and phase 19 results. Use this composed measurement as both prototypes' baseline, including retention-transaction starts that preparation retries discard.
3. **Estimates.** For each route, on the five binaries, measure on a prototype of one representative test per binary and extrapolate: process starts by origin, and Mac time per test and per binary. Send the table to the showrunner, then wait for its pick. This step builds nothing beyond the prototypes.
4. **Build the picked route.** Route 1 keeps a real-CLI test for each command. Route 2 keeps every ref and config form Git supports and the typed difference between missing and failed, falling back to Git where it cannot. Adding `gix` is the showrunner's pick under route 2.
5. **Proof.** One cold full Mac run passes every test with each under 1.000 s; a cold quiet natedev run passes with each under 1.000 s.
6. **Phase 19 lock holds at quiet load.** Source: the showrunner, 2026-10-19: "the quiet-machine re-measure (20 paired runs at load 16 or less) goes to phase 22." Phase 19's accepted busy reconcile evidence was p95 52.4 ms, maximum 68.5 ms; it did not establish the quiet-load requirement. At load 16 or less, run 20 paired measurements for each of the six paths with phase 19's method (its notes are in `/tmp/claude/unit/a433b49e-d462-472a-9f5b-3e767f5be8b6/phase19-lock-holds.md`), record the complete table in this phase's summary, and require every path's p95 under 100 ms. Find a miss's cause at its measured source, ordered retention-ref work included, and fix it in this phase.

**Files:**
- `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/gate.rs`, `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/parallel_phases.rs`, `crates/cargo-berth/tests/drift.rs` — the five slowest binaries
- `crates/cargo-berth-test-support/src/`, `crates/cargo-berth-test-support/Cargo.toml` — shared fixtures and the in-process runner (route 1)
- `crates/cargo-berth/src/lib.rs`, `crates/cargo-berth/src/main.rs`, `crates/cargo-berth/src/cli.rs`, `crates/cargo-berth/Cargo.toml` — route 1 only (`lib.rs` is new): cargo-berth is a binary whose `main.rs` calls `cli::Cli`, so the in-process runner needs a library boundary. The prototype shows parallel-safe calls that share no mutable working directory or environment between tests, and keeps real-CLI coverage.
- `crates/cargo-berth/src/git/`, `Cargo.toml`, `Cargo.lock` — route 2 (with `crates/cargo-berth/Cargo.toml` above): status, worktree list and ref reads through the picked `gix` crates
- `crates/cargo-berth/tests/process_start_budget.json` — the start pin, lowered to the new count
- `crates/cargo-berth/src/ledger/`, `crates/cargo-berth/src/git/refs.rs`, `crates/cargo-berth/src/reconcile.rs` — only if the quiet lock-hold run (Spec 6) misses, at the measured source

**Workers:** 1 writer, `mac`.

**Constraints:** As phase 18: no thread-count change, raised limit, longer timeout, skipped or `#[ignore]`d test, `cfg(target_os = "macos")` branch, nextest filter, or suite `TMPDIR` setting. Mac runs: one `mac_test.py claim` per run, released when it ends. A natedev run at a load above 16 of 32 is untrusted.

**Constraints from prior phases:** Phase 19 prepares observations outside the ledger lock and checks replay generation and session mappings under it. Keep the bounded re-preparation and its exhaustion error; never retry a durable append because a later ref action failed. A replaced journal source triggers a full replay, with no new user-facing state. The phase 19 invalidation and compaction tests stay authoritative. Phase 20 makes a stale-lock retention failure after the append a deferred outcome; phase 21 makes the covered-integration decision per holder.

**Acceptance gate:** The estimate table went to the showrunner and its pick is recorded in this Work Order before the build. `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` pass. The cold full Mac run passes with every test under 1.000 s. A cold quiet natedev whole-package run also passes with every test under 1.000 s, with load recorded and no sample accepted above load 16. The quiet paired lock-hold run shows every path under 100 ms at p95. A fresh completed whole-suite process report, supplied through `CARGO_BERTH_PROCESS_COUNT_REPORT`, passes its budget audit: it counts setup, tests and retry starts by origin, has no unresolved product origin, and meets the lowered budget. A package pass without that report is not process-budget evidence.

### Phase 23 — A cargo-berth test run leaves no repository in `/tmp`  · status: done

#### Work Order

**Depends on:** none. It runs beside phases 20 and 21 and checkpoints before phase 22. It has the number 23 because phase 22 was already planned and named to the showrunner.

**Goal:** Each repository that a cargo-berth test makes is removed when its test ends, also when the test fails or panics. A full test run adds no directory to `/tmp`.

**Spec:**
1. Source: the showrunner, 2026-10-19: "cargo-berth tests leak temp git repos into /tmp, and the leak is live now." It counted 423 `/tmp/.tmp*` directories made from 2026-10-18 12:00 until now, 13 to 72 each hour. They hold repositories with commits by "Berth Test" ("initial", "track berth configuration"), `.claude/` and `README.md`; some are empty. The start time agrees with the phase 18 and 19 fixture changes (`repository_template.rs`, `board/test_support.rs`, `restore_into`). Verbatim asks: "(1) find each leaking path — a TempDir kept with keep/into_path, moved into a killed child, or created outside the per-test TMPDIR … (2) fix it by construction, so a test repo cannot outlive its test; (3) add a red-first check that a test run leaves no new /tmp/.tmp* dir; (4) do not delete the leaked dirs."
2. **Find each leaker.** Run the whole `cargo-berth` package, setup script included, with a per-test `TMPDIR`: a nextest target runner (`--config 'target.x86_64-unknown-linux-gnu.runner="<script>"'`, never the `CARGO_TARGET_*_RUNNER` variable) that gives each test its own `TMPDIR` under a scratch root, records the test name, and removes the directory when it is empty after the test. Each directory that stays names one leaking test. Also find directories made outside the per-test `TMPDIR` (a fixed `/tmp` path, `temp_dir()` read before the runner sets it, the setup script, a child process given a cleared environment, a template build). Record each leaker with its cause in the summary: file, test or fixture, and why the directory outlives it.
3. **Fix by construction.** A fixture that makes a repository returns a value that owns it and removes it on drop, and nothing gives that ownership away: no `keep`, no `into_path`, no `mem::forget`, no path returned without its guard. A cache that must outlive a test process (a seed built once per test binary) lives under the build's target directory, never in `/tmp`. A test process that is killed and cannot unwind is out of scope: no measured leak came from one, and the realistic paths are return and panic (unit director, 2026-10-19, at review). Fix each leaker where it lives. Do not delete leaked directories, and do not add a cleanup sweep: a sweep hides a leak and does not stop it.
4. **Red-first check.** Add a test that fails while a leak is present and passes after the fix. It runs a fixture path that leaked (or the fixture builder with the defect shape) under a private `TMPDIR` and asserts that no directory stays when the fixture is dropped, the panic path included. Show it red on the unfixed tree, in the summary. Then show a full package run under the per-test runner with no directory left.
5. **Other leakers.** List the `/tmp/.tmp*` directories made since 2026-10-18 that are not cargo-berth's (about 70), grouped by what they hold and, where the contents name it, by the crate or tool that made them. Report them in the summary. Do not change those crates in this phase.

**Files:**
- `crates/cargo-berth-test-support/src/`, `crates/cargo-berth-test-support/Cargo.toml` — the repository template, integration repository and the guards that own each repository
- `crates/cargo-berth/src/board/test_support.rs`, `crates/cargo-berth/src/ledger/test_support.rs`, `crates/cargo-berth/src/git/fixture.rs` — unit-test fixtures
- `crates/cargo-berth/tests/support/` — shared integration-test fixtures
- `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/edges.rs`, `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/drift.rs`, `crates/cargo-berth/tests/parallel_phases.rs`, `crates/cargo-berth/tests/gate.rs`, `crates/cargo-berth/tests/answers.rs`, `crates/cargo-berth/tests/liveness.rs`, `crates/cargo-berth/tests/presentation.rs`, `crates/cargo-berth/tests/overlap.rs`, `crates/cargo-berth/tests/compaction.rs`, `crates/cargo-berth/tests/engine_instructions.rs`, `crates/cargo-berth/tests/front_end_corpus.rs`, `crates/cargo-berth/tests/recorded_templates.rs`, `crates/cargo-berth/tests/setup-recorded-templates.sh` — fixture call sites that leak
- `crates/cargo-berth/tests/temporary_directories.rs` — new: the red-first check

**Workers:** 1 writer, `test`, owning every file above.

**Constraints:** Runs beside phases 20 and 21: do not edit `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/src/drift/`, `crates/cargo-berth/src/git/refs.rs`, `crates/cargo-berth/src/git/error.rs`, `crates/cargo-berth/src/git/command.rs`, `crates/cargo-berth/src/git/mod.rs`, `crates/cargo-berth/src/gate/audit.rs`, `crates/cargo-berth/src/reconcile.rs`, `crates/cargo-berth/src/ledger/handle.rs`, `crates/cargo-berth/src/output.rs` or `crates/cargo-berth/src/verb/`. A leaker in one of those files is named to the unit director with its fix; it is not edited here. No deletion of any existing `/tmp` directory. No suite `TMPDIR` setting in nextest configuration: the per-test runner is a diagnostic tool only. No test may take a second. The process-start budget may only go down. Saved diagnostic output stays under a few GB; read each run and delete its output before the next.

**Constraints from prior phases:** Phase 18 templates are built once per build of a test binary under `CARGO_TARGET_TMPDIR` and copied into each test; keep that, and keep the `BeforeRun` setup-script build. Phase 19 prepares observations outside the ledger lock; the phase 19 invalidation and compaction tests stay authoritative.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` pass. The new check was red before the fix. A full package run under the per-test runner leaves no directory, and a count of `/tmp/.tmp*` before and after a full package run shows no new cargo-berth repository.
