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

### Phase 10 — Every refused check or claim leaves one typed journal record naming its requester, paths and holders  · status: todo

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


### Phase 12 — cargo-port's broken-worktree rows in the project list draw white text on the error red  · status: todo

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
- `crates/cargo-tile/themes/` (owner tile-fixes-unit; through berth)
- `crates/cargo-handler/themes/` (owner handler-enh-unit; through berth)

**Workers:** 1 writer.
- `impl` or whichever worker phase 10 leaves free — every file above, lint, and the package test.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` and `bash ~/.claude/scripts/delegate/verify.sh test cargo-port` green, with the item 5 test. Shots: the unit director's dark and light shots of the project list with a broken worktree row show white text on the error red.
