# berth-parallel

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** One unit director can hold one berth reservation per phase: a `check` under an unmapped session id opens a new reservation in the same coordination run, and the director's own Bash calls never widen one phase's reservation with another phase's files.

> **Production: remove-running-section** — unit `berth-parallel-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-15 14:53 EDT

One unit director should be able to run two phases side by side, each with its own reservation and checkpoint (produce.md's "parallel by default"). Today one director session plus one active reservation folds every later phase's files into that reservation (SessionMappingOrSingleActive, cli.rs:1088 check and :1389 drift), so they release with the first phase's checkpoint. Seen 2026-10-15 in tool-based-ui's info-panels plan, where Phases 16 and 20 had to wait for 15. The director runs check once per phase under CARGO_BERTH_SESSION_ID=<director sid>.phase-<N>, with paths split by each phase's Work Order. Two engine changes needed: (a) a check under a session id with no mapping opens a new reservation in the same coordination run instead of joining the run's only active one; (b) drift from the director's own Bash calls never widens one phase's reservation with another phase's files. Fix fast and install, test that the fix works first, robust testing after.

## Findings before planning

Read 2026-10-15 at 6d01f704.

- **Why a phase check joins the earlier phase.** `cargo-berth check` without `--reservation` parses to `CheckReservationSelection::SessionMappingOrSingleActive` (`cli.rs:1088`). Under the ledger lock, `select_first_touch_reservation_reuse` (`verb/claim.rs:1483`) sends both `FirstTouchSessionReservationMapping::AvailableSessionWithoutMapping` and `HarnessSessionUnavailable` (line 1531) to `from_unmapped_active_run_reservations` (line 524). With one active reservation that returns `SingleActiveRunReservation`, and the check widens it. So `CARGO_BERTH_SESSION_ID=<sid>.phase-16 cargo-berth check …` widens phase 15's reservation and maps `<sid>.phase-16` onto it (`session/mod.rs` `apply_journal_event` publishes on `Widen`). The new reservation path already exists: `NoActiveRunReservation` returns `AppendRequired`, and the caller appends a `Claim` in the same run (line 1455).
- **Identity resolution keeps the run.** `resolve_from_sources` reads the session mapping, then `CARGO_BERTH_RUN`, then the worktree marker. An unmapped phase session resolves through the marker to the director's run, which is what (a) needs.
- **Nothing else calls `check`.** `~/.claude` has no `cargo-berth check` caller. The pre-tool-use hook builds its own `CheckRequest` with `SessionMappingOrSingleActive` (`hook/pre_tool_use.rs:458`), so changing what bare `check` selects leaves the edit hook alone.
- **Why the director's Bash calls fold files.** The post-tool-use hook runs drift as `EveryActiveForPostCommit { widening: SessionMappingOrSingleCandidate }` (`hook/post_tool_use.rs:286`) under the payload's session id, the director's real session. `PostCommitWideningSelection::resolve_post_commit` (`drift/selection.rs:136`, lines 187–207) prefers that session's mapping. The director's session gets mapped as soon as any of its edits or Bash-observed writes widens a reservation. From then on every unheld changed path in the worktree widens the mapped phase, even with a second phase active.
- **Sibling-held paths are already safe.** `drift_coverage_probe` treats a path held by an active reservation of the same run and worktree as `NoForeignStanding`, so drift never widens one phase with a path the other phase's check declared. `partition_first_touch_protected_scopes` (`verb/claim.rs:1691`) does the same for `check`. The defect is only the unheld path and the selection above.
- **What must not change.** `session_mapping_attributes_post_commit_widening_with_two_active_reservations` (`tests/drift.rs:2785`) pins that a post-commit drift under an explicit `CARGO_BERTH_SESSION_ID` mapping widens the mapped one of two reservations. That is a phase session naming its phase, and it stays. `post_commit_drift_under_only_the_claude_code_session_does_not_adopt_it` (line 2814) already makes a commit under the Claude Code session ambiguous.

## Decisions made here

- **(a) is scoped to `check` with a usable, unmapped session id.** A new selection opens a new reservation; `HarnessSessionUnavailable` (a hand-run `check` with no session) keeps joining the run's only active reservation, and the edit hook keeps its selection. Risk: a bare `check` from a director's Bash under its real, unmapped session id now opens a reservation instead of joining one; no caller does that today.
- **A new phase reservation takes only paths no active sibling of the run holds.** It is the existing `partition_first_touch_protected_scopes` rule. A check whose every path a sibling holds reports `already_held` under that sibling and opens nothing. Overlapping phases order through the overlap answers, as produce.md says; phase 2 tests that path.
- **(b) is scoped to the post-tool-use hook.** With two or more active reservations of the acting run in the worktree, its drift ignores the session mapping and reports unheld paths as ambiguous, naming the candidates; with one, it widens that one as today. Explicit `CARGO_BERTH_SESSION_ID` drift and post-commit drift keep honouring the mapping, so a phase session can still attribute its own files. Risk: a director's hand-run bare `drift` under `CLAUDE_CODE_SESSION_ID` still follows its mapping; the director's protocol names the reservation instead.
- **Out of this unit:** `/unit:direct`'s `<BerthDecisions/>` forbids env-var prefixes on cargo-berth calls, and `/unit:checkpoint` runs bare `drift --full`. Running one check per phase under `CARGO_BERTH_SESSION_ID=<sid>.phase-<N>`, and checkpointing with `--reservation <id>`, is a skill change for the showrunner to place.

## Delegation Context

- **Project:** `cargo-berth`, the worktree reservation ledger and hook engine.
- **Project started:** 2026-10-15T19:00:00.632+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`. Every seat works only here, and every Work Order names it.
- **Base commit:** `6d01f70494ead626313d07ddd862e81190744ee9`.
- **Test lanes:** `crates/cargo-berth/tests/`.
- **Stack:** Rust workspace, cargo-nextest. The crate has no lib target: unit tests live in `cargo-berth::bin/cargo-berth` (`--bins`); integration tests in `crates/cargo-berth/tests/`.
- **Key files:**
  - `crates/cargo-berth/src/cli.rs` — `CheckArguments::into_check_request` (1085).
  - `crates/cargo-berth/src/verb/claim.rs` — `CheckReservationSelection` (397), `reported_session_mapping_publication` (1006), `FirstTouchReservationSelection` constructors (490–560), `select_first_touch_reservation_reuse` (1483), `reuse_first_touch_reservation` (1563), `partition_first_touch_protected_scopes` (1691).
  - `crates/cargo-berth/src/session/mod.rs` — `FirstTouchSessionReservationMapping` (243), `apply_journal_event`.
  - `crates/cargo-berth/src/drift/selection.rs` — `PostCommitWideningSelection` (126), `resolve_post_commit` (136).
  - `crates/cargo-berth/src/hook/post_tool_use.rs` — `post_tool_use_drift_request` (286).
  - `crates/cargo-berth/tests/lifecycle.rs` — `run_berth_with_session` (1793), session-mapping tests from 3047.
  - `crates/cargo-berth/tests/drift.rs` — `claim_with_session`, `post_commit_drift_with_session`, the two tests at 2785 and 2814.
  - `crates/cargo-berth/tests/hooks.rs` — post-tool-use tests from 2957; `post_tool_use_runs_drift_under_the_payload_session_not_the_ambient_one` (3429).
  - `docs/cargo-berth/as-built/worktree-coordination.md` — line 268 states check's selection; `json-contract.md` 721–762 states the ambiguous check outcome.
  - `crates/cargo-berth/CHANGELOG.md` — `## [Unreleased]`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - Identity resolves once per invocation through `ledger::resolve_identity`; the session mapping outranks `CARGO_BERTH_RUN` and the marker (as-built `worktree-coordination.md`).
  - A mapping naming another run never redirects a widen (`from_locked_session_mapping` doc).
  - No test takes a second (memory `no-multi-second-tests`). Format with `cargo +nightly fmt` only. One `verify.sh` at a time.
  - Hook lines are accurate for their case and stated once (memory `hooks-speak-only-when-actionable`).

## Phases

### Phase 1 — One reservation per phase session, and the director's Bash calls fold nothing  · status: done

#### As-built

- A bare `check` parses to `CheckReservationSelection::SessionMappingOrNew` (`cli.rs`). A valid session mapping selects its reservation. A usable session id with no mapping opens a new reservation in the acting coordination run (`append_unheld_first_touch_scopes`, `verb/claim.rs`). Without a usable session id, the check still joins the run's only active reservation in the worktree. The edit hook keeps `SessionMappingOrSingleActive`.
- A new phase reservation takes only paths no active same-run sibling holds (`partition_first_touch_protected_scopes`). When siblings hold every path, the check reports `already_held` under the sibling holding the first path and writes no mapping (`AlreadyHeldSessionMapping::PreserveUnmappedSession`). It then reports `session_mapping_publication = {"status": "left_unmapped"}` (`SessionIdentityMappingPublication::LeftUnmapped`), so a later bare check under that session opens new work. A mapped or explicit selection that reports `already_held` publishes its selected reservation.
- The post-tool-use hook runs drift with `PostCommitWideningSelection::SoleRunCandidate`. It widens only the run's sole active reservation in the worktree. With several active, unheld changed paths come back ambiguous with the candidate ids, even when the hook session maps to one of them. Explicit drift and post-commit drift keep `SessionMappingOrSingleCandidate`, so a phase session's mapping still attributes its own files.

**Files:**
- `crates/cargo-berth/src/cli.rs` — bare `check` selection
- `crates/cargo-berth/src/verb/claim.rs` — new-reservation selection, `AlreadyHeldSessionMapping`
- `crates/cargo-berth/src/session/mod.rs`, `src/output.rs` — `LeftUnmapped` publication and its message
- `crates/cargo-berth/src/drift/selection.rs`, `src/hook/post_tool_use.rs` — `SoleRunCandidate`
- `crates/cargo-berth/tests/parallel_phases.rs` — two-phase end-to-end test and the `left_unmapped` test
- `docs/cargo-berth/as-built/worktree-coordination.md`, `json-contract.md`, `docs/cargo-berth/generated/output-contract.json` — the selection rules and the `left_unmapped` wire value

**Binds later work:** `parallel_phases.rs` holds the helpers `initialized_repository`, `run_berth_with_session`, `run_post_tool_use` and `journal_events`.

**Gotchas:** running one check per phase under `CARGO_BERTH_SESSION_ID=<sid>.phase-<N>`, and checkpointing with `drift --full --reservation <id>`, are director-skill changes owned outside this crate.

### Phase 2 — Robust tests  · status: done

**Depends on:** Phase 1

#### As-built

- Acceptance tests pin both phase-1 changes: `lifecycle.rs` holds the single-phase lifecycle case; `parallel_phases.rs` holds the CLI cases (mixed sibling-held requests, the reservation limit, checkpoint drift beside a released sibling); `hooks.rs` holds the post-tool-use cases.
- A new phase reservation whose request includes paths an active same-run sibling holds reports them in `acquisition.already_held: Vec<SiblingHeldScopes>` (`sibling_held_scopes`, `append_first_touch_claim` in `verb/claim.rs`); the field is absent when empty.
- A bare `check` that would open a reservation past `maximum_reservations` returns `CheckPayload::ReservationLimitReached { maximum }` through `OutputEnvelope::check_reservation_limit_reached`, exit 1, instead of claim's invalid-input refusal.
- Full checkpoint drift never adopts a released sibling's committed paths: `unclaimed_path_can_widen` and `released_sibling_checkpoint_covers` (`drift/classification.rs`) with `compares_to_checkpoint` (`drift/observation.rs`) skip a path when the full comparison target equals the released sibling's protected tip. Dirty edits and later commits still widen.
- `RefusedRunPathEntry::HoldersEntered` is boxed; the check refusal summary constant is `CHECK_REJECTED_REQUEST_SUMMARY`.

**Files:**
- `crates/cargo-berth/tests/lifecycle.rs`, `tests/parallel_phases.rs`, `tests/hooks.rs` — the acceptance cases
- `crates/cargo-berth/src/verb/claim.rs`, `src/output.rs` — `already_held` and the typed limit refusal
- `crates/cargo-berth/src/drift/classification.rs`, `src/drift/observation.rs` — the released-sibling guard
- `docs/cargo-berth/as-built/json-contract.md`, `worktree-coordination.md`, `docs/cargo-berth/generated/output-contract.json`, `crates/cargo-berth/CHANGELOG.md` — the wire contract

**Binds later work:** `parallel_phases.rs` also holds `open_phase`, `phase_board`, `session_mappings`, `SESSION_MAPPING_PATH` and `CONFIGURATION_PATH`. A named claim reports `already_held` and the limit the way check does.

**Gotchas:** hooks-band test times swing from about 0.3 s to 2–3 s under machine load, unchanged tests included.

### Phase 3 — Answer an overlap for one named phase  · status: done

**Depends on:** Phase 2

#### As-built

- `cargo-berth claim <paths> --reservation <id> --<before|after|defer|override> <holder> --overlap-why "<why>"` records the answer on reservation `<id>`, which must be an active reservation of the acting run and worktree. `ClaimRequest` carries `ClaimReservationSelection::{NewReservation, Existing(ReservationId)}`; without `--reservation`, `claim` opens a new reservation as before.
- With `Existing`, `--before`, `--after` and `--override` widen the named reservation and attach the edge to it alone. `--defer`, or a request the named reservation already holds in full, records an answer-only `Answer` journal event (acquisition kind `answered`) that survives replay and maps the session to the named reservation. `--replace` retires only the named reservation's edges.
- The selected reservation keeps its recorded target, phase-start head, purpose and plan/phase; creation-only arguments never overwrite them. Sibling-held paths report in `acquisition.already_held`, and widening at `maximum_reservations` succeeds.
- An unknown, inactive or foreign-run id refuses with exit 5 and changes no journal entry or session mapping. A repeated same-direction `--after`/`--before` on the named reservation refuses with `duplicate_ordering_edge`, exit 2, before the journal write.
- The board's replaced-edge view reads `Answer` events.

**Files:**
- `crates/cargo-berth/src/cli.rs` — `--reservation` on `ClaimArguments`
- `crates/cargo-berth/src/verb/claim/mod.rs` — the claim verb (formerly `verb/claim.rs`)
- `crates/cargo-berth/src/verb/claim/named.rs` — named-selection validate, acquire and consume
- `crates/cargo-berth/src/output.rs`, `src/output_contract.rs`, `docs/cargo-berth/generated/output-contract.json` — widened and answered acquisition payloads
- `crates/cargo-berth/src/board/answers.rs` — replaced edges from `Answer` events
- `crates/cargo-berth/tests/parallel_phases.rs` — the named-claim tests
- `docs/cargo-berth/as-built/json-contract.md` — `duplicate_ordering_edge`, the `answered` kind and the selection refusals

**Gotchas:** replay rejects a `DuplicateEdge`, so a named claim must refuse a repeated edge before writing the journal. Named-claim fixtures ran 1–2.5 s under load until trimmed to at most 0.566 s.

**Ruled out:** deduplicating repeats on a plain claim — each plain claim opens a distinct reservation, so no duplicate edge arises.

### Phase 4 — An overlap refusal names only the holder  · status: todo

**Depends on:** Phase 3

#### Work Order

**Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`.

**Goal:** a blocked `check` or `claim` message names the holder's coordination run beside the holder, so nothing in it reads as the requester (showrunner, 2026-10-15).

**Spec:**
1. One holder: `Reservation <H> (coordination run <run>) on <branch> (<source>, <purpose>) holds paths that overlap the requested paths; reduce the requested scopes or coordinate with the holder, then retry.` Today it ends `holds overlapping paths for <run>`, where `<run>` is the holder's own run id and reads as the asking reservation.
2. Several holders: `<n> reservations hold paths that overlap the requested paths: reservation <H> (coordination run <run>) on <branch> (<source>, <purpose>); …; reduce the requested scopes or coordinate with the holders, then retry.` The run id moves beside each holder the same way.
3. Tests, each under 1 s: the one-holder `check` message equals the exact Spec 1 text for a real foreign holder (its reservation id, its run id, its branch), and the requester's run id appears nowhere in it; the several-holder `claim` message carries each holder's `(coordination run <run>)` beside its id. Update the existing assertion `2 reservations hold overlapping paths` in `tests/overlap.rs` to the new text.

**Files:**
- `crates/cargo-berth/src/output.rs` — `overlap_holder_description` (~4134)
- `crates/cargo-berth/tests/overlap.rs` — the Spec 3 tests and the updated assertion (~1317)
- `crates/cargo-berth/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — `output.rs` and the CHANGELOG.
- `test` — the Spec 3 tests and the updated assertion in `crates/cargo-berth/tests/overlap.rs` (test owns this file alone).

**Constraints from prior phases:**
- `ReservationConflict` carries `reservation_id`, `holder_run_id`, `holder_branch()`, `source` and `purpose`; the description helpers are `source_description` and `purpose_description` in `output.rs`.
- `tests/overlap.rs` already builds foreign holders with `foreign_worktree`, `make_dirty`, `reconcile_fixture` and `run_berth`, and reads the envelope with `json_output`.

**Acceptance gate:** `verify.sh lint cargo-berth` and `verify.sh test cargo-berth` green; every new test under 1 s. The checkpoint carries a `Change-Review:` line for the new wording.
