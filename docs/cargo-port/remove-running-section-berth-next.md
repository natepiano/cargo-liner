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

### Phase 3 — A trial merge in progress is never read as this worktree's edit  · status: todo

**Depends on:** Phase 2 (shared CHANGELOG, `src/drift/`, as-built docs)

#### Work Order

**Goal:** While git records a pending operation in an enrolled worktree — a merge in progress (`MERGE_HEAD`, including `git merge --no-commit --no-ff <peer tip>`, the production's trial merge), or a cherry-pick or revert stopped with `CHERRY_PICK_HEAD`/`REVERT_HEAD` — no hook or drift check creates a first-touch reservation or raises an incursion for the changes it brought in, and after `git merge --abort` nothing remains.

**Spec:**
1. Repro (handler-enh, 2026-10-16): a trial merge in an enrolled worktree raised incursions on paths tile-fixes and berth-parallel held and created first-touch reservation 01a147a4. Drift stands aside only for a rebase: `git::rewrite_in_progress` (`src/git/discovery.rs:118`) checks only `rebase-merge`/`rebase-apply` (`src/git/constants.rs:164-167`), used by `comparable_worktree` (`src/drift/execution.rs:510`, check at `:527`), which returns `DeferredPendingRewrite`, and `prepare_worktree_comparison` (`:534-550`) turns it into an unchanged empty report. `MERGE_HEAD`, `CHERRY_PICK_HEAD` and `REVERT_HEAD` sit in `OPERATION_IN_PROGRESS_MARKERS` (`src/worktree/constants.rs:10-16`), read only by enrollment (`src/worktree/enrollment.rs:343`). The post-tool-use hook (`src/hook/post_tool_use.rs:229`) runs drift, whose `claim_post_write_paths` (`execution.rs:552`, called at `:331` and `:398`) acquires the first touch.
2. Drift defers whenever git records a pending operation — rebase, merge, cherry-pick, revert — through one shared marker set and one semantic type naming which operation is pending, used by drift, enrollment and rewrite reconciliation; never a second list. The deferred report is the same unchanged empty report a pending rebase gives today, so no first touch and no incursion. The guarantee covers only operations git leaves observable state for: a cherry-pick or revert run with `--no-commit` that completes writes no `CHERRY_PICK_HEAD`/`REVERT_HEAD` and leaves only a dirty tree, which drift reads as this worktree's edit by design, since git records no pending operation.
3. The deferral stays in `comparable_worktree`, inside `prepare_drift_execution` (`execution.rs:208`), ahead of `classify_foreign_paths_before_lock` (`execution.rs:440`) and its `provenance::read_committed_foreign_paths` (`src/drift/provenance.rs:118`), so a pending operation causes no upstream lookup and no committed-history read.
4. Rewrite reconciliation keeps its rebase-only deferral: `reconcile::prepare_rewrite_reconciliation` (`src/reconcile.rs:1854`, filter at `:1863`) reads the same shared operation type and skips a pending branch-rewrite marker only when that type names a rebase. Drift and enrollment defer for every pending operation.
5. Once the operation ends, drift resumes: after an abort the tree is clean and nothing is reported; after a completed merge commit the merged-in commits are judged by phase 2's committed-origin rules.
6. Tests: (a) a trial merge of a peer's held paths, then a post-tool-use check: no reservation, no incursion; (b) a `git cherry-pick` of a peer commit that stops on a conflict (so git writes `CHERRY_PICK_HEAD`), then a post-tool-use check: no reservation, no incursion; (c) after `git merge --abort` the next check reports nothing; (d) a merge completed with a commit is judged as a commit, not deferred forever; (e) extend `drift_reports_no_change_while_a_rewrite_is_pending` (`src/drift/execution.rs:984`) to each operation; (f) a unit test that the shared operation type names a rebase only for `rebase-merge`/`rebase-apply`, so rewrite reconciliation still processes a branch-rewrite marker while a merge is pending. The integration tests (a)–(d) reach each state through real git commands, never a written marker file.
7. Docs: CHANGELOG `[Unreleased]` `Fixed` line; `docs/cargo-berth/as-built/worktree-coordination.md` (Drift ~374-406, invariants ~516-517), `rebase-settlement.md` (~128), `worktree-enrollment.md` (~85, 93, 113). The docs state that a completed `--no-commit` cherry-pick or revert reads as this worktree's edit.

**Files:**
- `crates/cargo-berth/src/git/discovery.rs`, `crates/cargo-berth/src/git/constants.rs`, `crates/cargo-berth/src/worktree/constants.rs`, `crates/cargo-berth/src/worktree/enrollment.rs` — the shared in-progress operation type
- `crates/cargo-berth/src/drift/execution.rs` — deferral and its unit test
- `crates/cargo-berth/src/reconcile.rs` — `prepare_rewrite_reconciliation` reads the shared type and keeps its rebase-only deferral
- `crates/cargo-berth/tests/` — the Spec 6 integration tests
- `crates/cargo-berth/CHANGELOG.md`
- `docs/cargo-berth/as-built/worktree-coordination.md`, `docs/cargo-berth/as-built/rebase-settlement.md`, `docs/cargo-berth/as-built/worktree-enrollment.md`

**Workers:** 1 writer + 1 tester
- `impl` — `src/` (the unit tests included), CHANGELOG, as-built docs; runs lint and the full gate.
- `test` — the Spec 6 integration tests in `tests/`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; a trial merge in an enrolled worktree leaves the board unchanged.

### Phase 4 — Claiming a new path behind a holder you already sequenced against succeeds  · status: todo

**Depends on:** Phase 3 (shared CHANGELOG and as-built docs)

#### Work Order

**Goal:** `cargo-berth claim <new path> --reservation R --before H` (or `--after H`), when R already has that ordering edge with H, adds the path to R under the existing edge and exits 0.

**Spec:**
1. Repro (handler-enh, 2026-10-16): reservation 01a14740 claimed `crates/tui_pane/Cargo.toml` and `CHANGELOG.md` with `--before 01a13687`; a second `claim crates/cargo-handler/CHANGELOG.md --reservation 01a14740… --before 01a13687…` exits 2 `duplicate_ordering_edge`, the path is never added, and `check` on it stays `blocked_by_overlap`. Today `bind_claim_authorization` (`src/verb/claim/mod.rs:2336-2351`) refuses when `ordering_graph.contains_sequence(requester, blocker)` (`src/edge/graph.rs:365`), before `coverage()`/`mutation()` (`src/verb/claim/named.rs:195-196`) add any path.
2. When the edge in the requested direction already exists, the claim widens R with the new paths and binds their overlaps with H to that existing edge, recording no second edge: graph replay refuses a repeated endpoint pair (`add_edge`, `graph.rs:660-669`), and coverage is per scope (`partition::reservations_authorize_scope`, `src/reservation/partition.rs:158-186`), so the new scopes need a recorded authorization naming the existing edge. Record it as a `ConflictAuthorization` form that carries the existing edge's identity (`EdgeId`), its direction and its recorded reason, and names the newly approved scopes — a semantic type, never an optional edge. It is not `ConflictAuthorization::ExistingAnswersCoverEveryOverlap` (`src/answer/conflict_authorization.rs:111`, used by drift widen at `src/reservation/retention.rs:627`): that form means earlier answers already authorize every scope, and the board records no answer for it (`board/answers.rs:561-565`). With several holders, only the holders whose edge is new add edges; a holder whose edge exists counts nothing toward the edge limit (`mod.rs:2353-2365`). Journal replay of existing ledgers is unchanged; the new form's replay and its `JournalOperation` widen classification (`src/ledger/journal.rs:798-806`) follow the edge-bound forms.
3. The board renders the new authorization: `board/answers.rs::append_authorization_answer` (`src/board/answers.rs:459`) records one answer row bound to the existing edge, with its identity, direction and reason, naming the newly approved scopes.
4. A claim whose paths the existing edge's recorded answers already cover is an already-satisfied claim. The claim's committed-action validation returns a successful outcome with no journal append: add one variant for it to `CommittedActionValidation` (`src/ledger/handle.rs:172`) and its matching `LedgerCommittedActionOutcome` variant (`handle.rs:243`), handled in `transact_with_committed_action_and_consume_locked_outcome` (`handle.rs:407`) — one variant each, never an `Option` or a `bool`. Today `named::acquire` (`src/verb/claim/named.rs:55`) always returns `Append`, and `mutation` (`named.rs:290`) writes a `JournalOperation::Answer` even with no scope growth. Session publication is separate from journal idempotence: `consume` (`named.rs:113`) still publishes the requested reservation as the session's selection for an already-satisfied claim, as it does for `AlreadyHeld` and `Answered` today.
5. The opposite direction to an existing edge is still refused as today. `--defer` and the `sequence` verb are unchanged; `OutputStatus::DuplicateOrderingEdge` stays for the `sequence` verb (`SequenceRejectionKind::Duplicate`, `src/output.rs:1417-1421`). Remove `ClaimRejection`/`ClaimError::DuplicateOrdering`, `OutputEnvelope::claim_duplicate_ordering` (`src/output.rs:2581-2587`) and its caller (`src/verb/claim/mod.rs:2986`) if nothing else reaches them.
6. Tests: invert `assert_repeated_named_ordering_is_refused` (`tests/parallel_phases.rs:1430`, callers at :822 and :827) — the `file:extra.rs` case widens and succeeds and `check` on it is clear; the already-satisfied repeat succeeds, leaves the journal file byte-identical, and still publishes the session's reservation selection; both for `--before` and `--after`. Add: two holders where one edge exists and one is new; the opposite direction is still refused; replay after the widen succeeds; in `tests/board.rs`, after the widen the board's overlap answers hold one answer bound to the existing edge, with its identity, direction and reason, naming the new scopes. Keep the `sequence` verb's duplicate tests (`tests/edges.rs:808-812`, `:2290-2296`).
7. Docs: CHANGELOG `[Unreleased]` `Fixed` line; `docs/cargo-berth/as-built/json-contract.md:860-865`, `worktree-coordination.md:332`, `phase-reservations.md:105`.

**Files:**
- `crates/cargo-berth/src/verb/claim/` — the refusal, authorization binding, widen and the already-satisfied outcome
- `crates/cargo-berth/src/answer/conflict_authorization.rs`, `crates/cargo-berth/src/edge/graph.rs`, `crates/cargo-berth/src/ledger/journal.rs` — authorization naming an existing edge, and its replay
- `crates/cargo-berth/src/ledger/handle.rs`, `crates/cargo-berth/src/ledger/mod.rs` — the successful no-append validation outcome and its exports
- `crates/cargo-berth/src/board/answers.rs` — the board answer row bound to an existing edge
- `crates/cargo-berth/src/output.rs` — remove the claim-only duplicate envelope
- `crates/cargo-berth/tests/` — the Spec 6 tests
- `crates/cargo-berth/CHANGELOG.md`
- `docs/cargo-berth/as-built/json-contract.md`, `docs/cargo-berth/as-built/worktree-coordination.md`, `docs/cargo-berth/as-built/phase-reservations.md`

**Workers:** 1 writer + 1 tester
- `impl` — `src/` (`board/answers.rs` included), CHANGELOG, as-built docs; runs lint and the full gate.
- `test` — the Spec 6 tests in `tests/`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; the repro's second claim exits 0 and `check` on the new path is clear.

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
6. Docs: README "Board" (~382–420) states the live refresh and that the raw diagnostics are in `board --json`, not the terminal board; CHANGELOG `Fixed` line; `docs/cargo-berth/as-built/parallel-phases.md` (~96–108, 137, and the known-defect lines ~151–152) and `worktree-coordination.md` (~434) describe the refresh and the Overview that shows only the table.

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
2. Compute both from the same report the lifecycle read already reconciles (`execute_reservation_lifecycle` → `reservation_lifecycle_snapshot`), through the helpers `BoardModel::build` uses (`declared_ordering_constraints`, `board_overlap_answers`), filtered to the reservation — one derivation for both board answers, not a second copy. When extracting the shared answer projection, replace two `Option`s with semantic outcome enums: `board/answers.rs::reservation_answers` (`src/board/answers.rs:442`), whose `Option<(ReservationId, &[ConflictAuthorization])>` conflates an operation that records no answer with an answer that has no reservation binding, gets one variant for each; and `DeferralConsequence::from_lifecycles` (`src/board/rows.rs:462`), whose `None` means both endpoints have released, returns an enum with a variant for that case. Every wire layout stays unchanged.
3. The JSON envelope's rendered `presentation` (`reservation_lifecycle_presentation`) lists the edges and answers after the lifecycle details. The selector still requires `--json` (`worktree-coordination.md:436`).
4. Regenerate the output contract; follow the versioning rule in `docs/cargo-berth/as-built/json-contract.md` for an added payload field.
5. Tests in `tests/board.rs`: (a) a successor with a declared `--after` edge reports that edge from both the predecessor's and successor's `board --reservation` answers, for a waiting edge and a settled one; (b) an edge or answer between two other reservations is absent; (c) a reservation with a recorded overlap answer reports it, whether it asked or was answered about, and a released answer from history is not reported as live; (d) a reservation with neither reports empty arrays; (e) each entry equals the full board's entry for the same edge or answer, including an answer bound to an existing edge by a claim that widened behind it (phase 4), compared between the full board and both endpoints' `board --reservation` answers; (f) extend `reservation_report_and_presentation`'s exact presentation assertion (~2094); (g) unit tests in `src/board/answers.rs` and `src/board/rows.rs`: an answer with no reservation binding, and an answer whose endpoints have both released, never become live reservation answers. Extend `reservation_report`'s helper rather than adding a second one.
6. Docs: json-contract.md "Named reservation lifecycle" (~560–620) and worktree-coordination.md (~436) describe the new fields; CHANGELOG `Fixed` line.

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
