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

**Depends on:** none

#### Work Order

**Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`.

**Goal:** the two engine changes, proved by one end-to-end test of the 2026-10-15 shape, then installed. The fast fix the user asked for first.

**Spec:**

1. **`CheckReservationSelection::SessionMappingOrNewReservation`.** New variant: "Use a valid harness-session mapping; a usable session id with no mapping opens a new reservation in the acting run; with no session id, the acting run's only active reservation." `CheckArguments::into_check_request` uses it when `--reservation` is absent. The pre-tool-use hook keeps `SessionMappingOrSingleActive`. `reported_session_mapping_publication` treats it as `SessionMappingOrSingleActive`.
2. **`select_first_touch_reservation_reuse`.** Under the new selection, `AvailableSessionWithoutMapping` returns `AppendRequired` with the protected scopes left after removing every path an active sibling of the acting run and worktree holds (reuse the partition rule; factor a reservation-free form of `partition_first_touch_protected_scopes` if needed). When nothing is left, report `already_held` under the sibling that holds the first path, exactly as the existing reuse does, and publish no mapping move. `Mapped` and `HarnessSessionUnavailable` behave as today. The appended `Claim` publishes the phase session's mapping through `apply_journal_event`, unchanged.
3. **`PostCommitWideningSelection::SoleRunCandidate`.** New variant: "Widen the acting run's only active reservation in this worktree; with several, attribute nothing and name them." `post_tool_use_drift_request` uses it. In `resolve_post_commit` it is `SessionMappingOrSingleCandidate` without the mapping arm: `[]` → `NotNeeded`, `[one]` → `Selected`, more → `Ambiguous(candidates)`; an unidentified run → `NotNeeded`. The post-commit git hook keeps `SessionMappingOrSingleCandidate`.
4. **End-to-end test** `a_director_runs_two_phases_each_with_its_own_reservation` in a new `crates/cargo-berth/tests/parallel_phases.rs` (copy the small session and hook helpers it needs from `tests/lifecycle.rs` and `tests/hooks.rs`), under 1 s:
   - Director session `dir-sid`. `check file:a.rs` under `CARGO_BERTH_SESSION_ID=dir-sid.phase-15` → reservation R15.
   - `check file:b.rs` under `dir-sid.phase-16` → a different id R16, same coordination run; R15's scopes are still only `a.rs`.
   - Map `dir-sid` onto R15 (a `check --reservation R15` under `dir-sid`), write unheld `c.rs`, run `hook post-tool-use` with a Bash payload whose `session_id` is `dir-sid`. No `widen` event; the drift answer names both R15 and R16 as candidates for `c.rs`.
   - Release R15; R16 is still active and holds `b.rs`.
5. **Docs.** `worktree-coordination.md` line 268: state the three selection cases of bare `check`. `json-contract.md` 721: the ambiguity now arises only with no usable session id. CHANGELOG `### Changed`: one entry for each change, in user terms.

**Files:**
- `crates/cargo-berth/src/cli.rs` — bare `check` uses the new selection
- `crates/cargo-berth/src/verb/claim.rs` — `CheckReservationSelection::SessionMappingOrNewReservation` and its locked selection
- `crates/cargo-berth/src/drift/selection.rs` — `PostCommitWideningSelection::SoleRunCandidate`
- `crates/cargo-berth/src/hook/post_tool_use.rs` — the hook's drift request uses it
- `crates/cargo-berth/tests/parallel_phases.rs` — new end-to-end test
- `docs/cargo-berth/as-built/worktree-coordination.md`, `docs/cargo-berth/as-built/json-contract.md` — the selection rules
- `crates/cargo-berth/CHANGELOG.md` — two Changed entries

Tests whose expectation was the old join (unmapped session joining the sole reservation through `check`) are updated to the new rule, each named in the phase report.

**Workers:** 1 writer + 1 tester. The new test is written from the Spec alone, beside the engine change.
- `impl` — `src/cli.rs`, `src/verb/claim.rs`, `src/drift/selection.rs`, `src/hook/post_tool_use.rs`, the two as-built docs, `CHANGELOG.md`, and edits to existing tests the new rule breaks.
- `test` — the new `crates/cargo-berth/tests/parallel_phases.rs` holding Spec item 4, run against the base tree first to show it fails.

**Constraints from prior phases:** none; this is the first phase.

**Acceptance gate:**
- The end-to-end test fails at the base commit and passes after (the writer shows the failing run first).
- `verify.sh lint cargo-berth` and `verify.sh test cargo-berth` green; the two drift tests at 2785 and 2814 unchanged and passing.
- **Install.** After the showrunner merges the checkpoint, install cargo-berth from the merged commit on natedev and the Mac, as for berth-hook-hang, and confirm `cargo-berth --version` on both. The unit does it if the showrunner asks.

### Phase 2 — Robust tests  · status: todo

**Depends on:** Phase 1

#### Work Order

**Worktree:** `/home/natepiano/rust/cargo-liner-berth-parallel`, branch `remove-running-section-berth-parallel`.

**Goal:** pin every neighbouring case of the two changes, and fix any defect they surface in this phase.

**Spec — tests, each under 1 s, in the file that holds its neighbours:**

1. `check` with no session id and one active reservation still joins it; with two, still returns `ambiguous_active_run_reservations` (`tests/lifecycle.rs`).
2. `check --reservation R` under an unmapped phase session widens R and maps the session to R.
3. A second `check` under the same phase session widens its own reservation, never the sibling.
4. Overlap between phases: a phase-16 check naming a path R15 holds plus a free path → R16 holds only the free path, response states the held path under R15; a check naming only R15's paths opens nothing.
5. The edit hook (`hook pre-tool-use`) under an unmapped session with one active reservation still joins it.
6. Post-tool-use drift with one active reservation still widens it (regression of today's flow); with two and an unmapped session, ambiguous as before; with an unidentified run, nothing.
7. Phase checkpoint flow: `drift --full --reservation R16` after R15's paths were committed and R15 released widens R16 only with its own unclaimed files, not R15's committed ones.
8. Releasing R15 retires only the `<sid>.phase-15` mapping; `<sid>.phase-16` still selects R16.
9. The reservation limit counts phase reservations: at `maximum_reservations`, a new phase check returns `reservation_limit_reached`.
10. The hook's ambiguous-attribution line reads correctly for the parallel case (names the candidates and the recovery `drift --reservation <id>`), stated once.

**Files:**
- `crates/cargo-berth/tests/parallel_phases.rs` — cases 1–4, 7–9
- `crates/cargo-berth/tests/hooks.rs` — cases 5, 6, 10
- `crates/cargo-berth/src/` — only an engine file a failing test points at
- `docs/cargo-berth/as-built/worktree-coordination.md` — any rule a fix adds

**Workers:** 1 writer + 1 tester.
- `impl` — any engine file a failing test points at, and the as-built docs for any rule a fix adds.
- `test` — the ten tests, in `crates/cargo-berth/tests/parallel_phases.rs` and the neighbouring test files named per case.

**Constraints from prior phases:** maintained by phase review after Phase 1.

**Acceptance gate:** `verify.sh lint cargo-berth` and `verify.sh test cargo-berth` green; every new test under 1 s; the phase report lists each case with its test name. If an engine fix landed, install again as in Phase 1.
