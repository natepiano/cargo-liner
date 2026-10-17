# berth-next

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Initializing berth leaves the trunk worktree clean, the terminal board shows reservations created after it opens and shows no raw diagnostics, and a single-reservation board answer carries that reservation's ordering edges and overlap answers.

> **As-built disposition: amend** — `docs/cargo-berth/as-built/worktree-enrollment.md`, `docs/cargo-berth/as-built/parallel-phases.md`, `docs/cargo-berth/as-built/worktree-coordination.md`, `docs/cargo-berth/as-built/json-contract.md`, `docs/cargo-berth/as-built/operations.md`

> **Production: remove-running-section** — unit `berth-parallel-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-16, from the showrunner: the three berth-parallel next items (that file is retired; each item is a phase here), one phase each. Phase 2 was added the same day from organon's berth report (resolved incursions re-raised for commits already on `init/catalyst`); the showrunner placed it ahead of the board phases because it hits every session's hook. They are berth defects under the user's berth standing approval: fast fix with tests, install on both machines after each checkpoint.

## Delegation Context

- **Project:** `cargo-berth`, the worktree reservation ledger and hook engine.
- **Project started:** 2026-10-16T23:47:40.606+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`. Every seat works only here, and every Work Order names it.
- **Base commit:** `b2d6f65d` (main).
- **Stack:** Rust workspace, cargo-nextest, ratatui + crossterm for the terminal board. The crate has no lib target, so unit tests live in `cargo-berth::bin/cargo-berth` and integration tests in `crates/cargo-berth/tests/`.
- **Test lanes:** `crates/cargo-berth/tests/`.
- **Key files:**
  - `crates/cargo-berth/src/config.rs` — `BerthConfig::initialize` (~151), `BerthConfig::relative_path` (~142), `CLAUDE_DIRECTORY` / `CONFIGURATION_DIRECTORY` / `CONFIGURATION_FILE` (17–19); doc comment at ~43 says the configuration file is untracked and per-worktree.
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
  - `crates/cargo-berth/tests/ledger.rs` — init tests; `clean_init_ignores_its_untracked_configuration_and_keeps_the_legacy_line` (444) asserts the untracked list equals `.claude/config/berth.toml`; `CONFIGURATION_PATH` (42).
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

## Phases

### Phase 1 — Initializing berth leaves no untracked `.claude/` in trunk  · status: done

**Depends on:** none

#### Work Order

**Goal:** After `cargo-berth init`, `git status --short` in the trunk worktree shows no `.claude/` entry, while the configuration stays at `.claude/config/berth.toml`.

**Spec:**
1. Every `init` (`initialize_ledger`, after `Ledger::initialize`, whether or not the configuration file already existed) makes git ignore that file for every worktree: append the line `/.claude/config/berth.toml` to the common `info/exclude` file (`git rev-parse --git-common-dir` + `info/exclude`; create `info/` when absent), once. A second `init` adds no second line, and an existing line (with or without the leading `/`) counts as present. A repository that tracks the file is unaffected: exclude never hides a tracked file.
2. The configuration path and contents do not change. Keep `observe_merge_working_tree`'s drop of `BerthConfig::relative_path()` (a repository initialized before this change has no exclude line until `init` runs again).
3. Write the exclude line through one place that owns the path string, built from `BerthConfig::relative_path()`, so the excluded path and the written path cannot diverge.
4. Tests: (a) after a clean `init`, `git status --short` and `git ls-files --others --exclude-standard` list nothing under `.claude/` — update `clean_init_ignores_its_untracked_configuration_and_keeps_the_legacy_line` (tests/ledger.rs:444), which asserts the opposite; (b) a second `init` leaves exactly one exclude line; (c) a linked worktree's `git status --short` shows no `.claude/` entry after `init`.
5. Docs: README "First use" init text (~28–57) says init excludes the file from git status; CHANGELOG `[Unreleased]` `Fixed` line; `docs/cargo-berth/as-built/worktree-enrollment.md` (lines ~5, 12, 18, 84, 112, 119, 141) and `operations.md` (~14–28, 84–86) state the exclude.

**Files:**
- `crates/cargo-berth/src/config.rs` — exclude path ownership and the exclude write
- `crates/cargo-berth/src/cli.rs` — `initialize_ledger` calls the exclude write on every `init`, including one that finds the configuration already present
- `crates/cargo-berth/tests/ledger.rs` — the Spec 4 tests
- `crates/cargo-berth/README.md`, `crates/cargo-berth/CHANGELOG.md`
- `docs/cargo-berth/as-built/worktree-enrollment.md`, `docs/cargo-berth/as-built/operations.md`

**Workers:** 1 writer + 1 tester
- `impl` — `src/`, README, CHANGELOG, as-built docs; runs lint and the full gate.
- `test` — the Spec 4 tests in `tests/ledger.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; after `init`, `git status --short` shows no `.claude/` entry.

### Phase 2 — An incursion from commits already on the integration target is never raised, and a resolved one stays resolved  · status: todo

**Depends on:** Phase 1 (shared README and CHANGELOG)

#### Work Order

**Goal:** A reservation whose committed range holds commits the integration target already reaches raises no incursion for them, and `resolve --every-incursion` stays resolved on the next hook check unless a new commit enters the path.

**Spec:**
1. Before classification, drop from a reservation's committed paths every commit the integration target reaches. Committed paths today come from a plain `phase_start..HEAD` diff (`src/drift/observation.rs:667`, `src/git/paths.rs:41-65`), so work merged in from the target counts as the phase's own. Classification raises an incursion on path and holder alone (`src/drift/classification.rs:418-419`); provenance (`src/drift/provenance.rs:391-421`) is computed only for the message. Move that commit-origin read (`commits_outside_origin_basis`) ahead of classification so a committed-only path whose in-range commits are all on the target is not an incursion.
2. Judge target membership against the target's upstream (for example `origin/init/catalyst`) as well as the local ref (`src/ledger/target.rs:36-52`), so a local target that is behind its upstream does not mark upstream commits as "this phase authored it".
3. An incursion answer covers its path and the commits that entered it: record the commit ids in each blocked path, so a holder set that grows later on the same commits (`src/reservation/retention.rs:885-913`, uncovered at ~854) does not open a new incident. Only a new commit into the path reopens it. Journal replay of existing incidents is unchanged (an old incident without commit ids keeps today's holder-subset rule).
4. Tests: (a) invert `tests/drift.rs:1528`, which asserts today's behaviour, and extend `merged_integration_target_commits_are_not_attributed_to_the_lane` (~1601); (b) a resolved incursion whose holder set grows stays resolved, next to `incursion_incident_round_trip_deduplicates_and_resolves` (~577) and `an_answered_incursion_covers_a_sibling_claimed_after_the_answer` (~1086); (c) a drift test where the local target is behind its upstream; (d) a `tests/hooks.rs` case with no STOP after `resolve --every-incursion`; (e) a new commit into a resolved path reopens it.
5. Docs: CHANGELOG `[Unreleased]` `Fixed` line; `docs/cargo-berth/as-built/worktree-coordination.md` and `parallel-phases.md` lines that describe incursion classification and resolution.

**Files:**
- `crates/cargo-berth/src/drift/` — classification, observation and provenance ordering
- `crates/cargo-berth/src/ledger/target.rs` — upstream-aware target membership
- `crates/cargo-berth/src/reservation/retention.rs` — commit-keyed incursion coverage
- `crates/cargo-berth/src/output.rs` — incursion message, if the commit list changes
- `crates/cargo-berth/tests/` — the Spec 4 tests
- `crates/cargo-berth/CHANGELOG.md`
- `docs/cargo-berth/as-built/worktree-coordination.md`, `docs/cargo-berth/as-built/parallel-phases.md`

**Workers:** 1 writer + 1 tester
- `impl` — `src/`, CHANGELOG, as-built docs; runs lint and the full gate.
- `test` — the Spec 4 tests in `tests/`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; a reservation with target-reachable commits in its range reports no incursion for them; after `resolve --every-incursion`, the next hook check raises none.

### Phase 3 — The terminal board shows reservations created after it opens, and its Overview holds no raw diagnostics  · status: todo

**Depends on:** Phase 2 (shared README and CHANGELOG)

#### Work Order

**Goal:** A reservation created while the board is open appears in the Overview and Reservations panes without a restart, and the Overview shows only its Reservations table.

**Spec:**
1. `event_loop` waits with `event::poll` and a 1-second timeout instead of a blocking `event::read()`. On each timeout it checks whether the ledger moved (the journal position the model was built at against the ledger's current one) with a read that does not reconcile; only when it moved does it rebuild: re-run the same `reconcile::reconcile` + `BoardModel::build` path `verb::board::execute` uses, rebuild the pane documents, and redraw. An unchanged ledger costs no reconcile.
2. `tui::run` takes a model source instead of one `&BoardModel`: a type owned by `verb/board.rs` (or `board/`) that holds the invocation directory and builds a fresh `BoardModel`. A rebuild that fails keeps the last good model on screen and shows the failure in the status line until the next good rebuild; it never exits the board.
3. A rebuild keeps the selected pane and each pane's scroll offset, clamped to the new document length.
4. The Overview pane shows only the Reservations table. The five `OVERVIEW_FIELDS` (`journal_position`, `recovered_bypasses_this_invocation`, `integration_order`, `targets`, `git_cost`) leave the terminal board entirely, with no new pane: `board --json` already carries them for anyone diagnosing (showrunner, 2026-10-16: fewer things on screen wins). `BoardPaneDocuments::from_model` drops those fields by name instead of failing on them as unassigned, so every other model field still has exactly one pane. Update `reassembled_model_value` (no Overview split at `"\n\n"`) and `every_model_fact_is_reachable_through_exact_pane_documents` (every model field except the five is reachable), and retire `recovered_bypasses_render_only_in_the_model_that_reports_them`, whose fact the board no longer renders, unless `board --json` lacks a test for it.
5. Tests (unit tests in `tui.rs`, `TestBackend`): (a) a model rebuilt with one more reservation renders that reservation in the Overview and Reservations panes; (b) no pane's text contains any of the five fields (`git_cost`, `journal_position`, …); (c) every other model field is reachable through exactly one pane; (d) a failed rebuild keeps the previous rows and shows the failure; (e) selection and scroll survive a rebuild, clamped. The loop reads events and ticks through a trait the tests implement, so no test sleeps.
6. Docs: README "Board" (~382–420) states the live refresh and that the raw diagnostics are in `board --json`, not the terminal board; CHANGELOG `Fixed` line; `docs/cargo-berth/as-built/parallel-phases.md` (~96–108, 137, and the known-defect lines ~151–152) and `worktree-coordination.md` (~434) describe the refresh and the Overview that shows only the table.

**Files:**
- `crates/cargo-berth/src/board/tui.rs` — event loop, refresh, panes, unit tests
- `crates/cargo-berth/src/verb/board.rs` — model source passed to `tui::run`
- `crates/cargo-berth/src/board/rows.rs` — only if the journal-position comparison needs an accessor
- `crates/cargo-berth/README.md`, `crates/cargo-berth/CHANGELOG.md`
- `docs/cargo-berth/as-built/parallel-phases.md`, `docs/cargo-berth/as-built/worktree-coordination.md`

**Workers:** 1 writer + 1 tester — nothing splits: the tests are unit tests inside `tui.rs`, so `impl` takes every file.
- `impl` — every file above, the Spec 5 tests included; runs lint and the full gate.
- `test` — opens as reviewer of the Spec 5 tests once `impl` posts them; writes nothing.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; a live board on natedev shows a reservation created after it opened within two seconds, and its Overview shows only the table.

### Phase 4 — A single-reservation board answer carries its ordering edges and overlap answers  · status: todo

**Depends on:** Phase 3 (shared README and CHANGELOG, `verb/board.rs`)

#### Work Order

**Goal:** `cargo-berth board --reservation <id> --json` includes that reservation's ordering edges and overlap answers.

**Spec:**
1. `ReservationReportSnapshot` (`src/output.rs` ~1058) gains `ordering_edges` and `overlap_answers`. `ordering_edges` holds every declared ordering constraint naming the reservation as predecessor or successor, waiting and settled, each in the same JSON shape the full board uses (`WaitingHold::OrderingEdge` fields, `SettledOrderingConstraint`); `overlap_answers` holds every live `RecordedOverlapAnswer` naming it. Empty arrays when none.
2. Compute both from the same report the lifecycle read already reconciles (`execute_reservation_lifecycle` → `reservation_lifecycle_snapshot`), through the helpers `BoardModel::build` uses (`declared_ordering_constraints`, `board_overlap_answers`), filtered to the reservation — one derivation for both board answers, not a second copy.
3. Text mode (`reservation_lifecycle_presentation`) lists the edges and answers after the lifecycle lines.
4. Regenerate the output contract; follow the versioning rule in `docs/cargo-berth/as-built/json-contract.md` for an added payload field.
5. Tests in `tests/board.rs`: (a) a successor with a declared `--after` edge reports that edge from both the predecessor's and successor's `board --reservation` answers; (b) a reservation with a recorded overlap answer reports it; (c) a reservation with neither reports empty arrays. Extend `reservation_report`'s helper rather than adding a second one.
6. Docs: json-contract.md "Named reservation lifecycle" (~560–620) and worktree-coordination.md (~436) describe the new fields; CHANGELOG `Fixed` line.

**Files:**
- `crates/cargo-berth/src/output.rs` — snapshot fields
- `crates/cargo-berth/src/board/report.rs` — snapshot derivation and text presentation
- `crates/cargo-berth/src/board/rows.rs`, `crates/cargo-berth/src/board/answers.rs` — shared per-reservation filtering
- `crates/cargo-berth/src/verb/board.rs` — lifecycle path
- `crates/cargo-berth/src/output_contract.rs`, `docs/cargo-berth/generated/output-contract.json`
- `crates/cargo-berth/tests/board.rs` — the Spec 5 tests
- `crates/cargo-berth/CHANGELOG.md`
- `docs/cargo-berth/as-built/json-contract.md`, `docs/cargo-berth/as-built/worktree-coordination.md`

**Workers:** 1 writer + 1 tester
- `impl` — `src/`, generated contract, CHANGELOG, as-built docs; runs lint and the full gate.
- `test` — the Spec 5 tests in `tests/board.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` green; `board --reservation <id> --json` on the live ledger shows `ordering_edges` and `overlap_answers`.
