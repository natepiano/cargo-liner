# berth-structure — next items

Approved work this plan does not cover. Each item names a concrete target and
what would satisfy it. Adding an item commits nobody to building it; scheduling
one into a phase is the decision.

## 1. A contradictory proposal token re-proposes rather than claiming

**Target:** `crates/cargo-berth/tests/` (a lifecycle or answers case).

A proposal token is the serialized proposal, answer included. Spending a `defer`
token under `--before` re-gates at exit 3 with a refreshed proposal rather than
landing the claim — verified by hand against two worktrees, covered by no test.
The control spends cleanly at exit 0 `claimed`. The property is enforced where
the token lives, so the test belongs beside the engine, not in a front end.

Satisfied by: a test that answers `defer`, captures the token, re-invokes with
`--before` and that token, and asserts exit 3 plus a proposal whose answer is
`before`.

## 2. The re-proposal does not say why it re-gated

**Target:** `crates/cargo-berth/src/output.rs`, the proposal rendering.

When a contradictory token is spent, the engine correctly re-proposes, but the
rendered text is an ordinary proposal. It never says the supplied token
contradicted the answer it was presented with, so a reader sees the same screen
twice and cannot tell that anything was rejected.

Satisfied by: the re-gated proposal naming the contradiction — the answer the
token carried and the answer that was requested — as its own presentation block.

## 3. A worktree added after `cargo-berth init` reports `unconfigured`

**Target:** `crates/cargo-berth/src/config.rs` and the worktree discovery path.

`.claude/config/berth.toml` is untracked and per-worktree, so a `git worktree
add` performed after `init` produces a worktree the engine calls `unconfigured`.
Every verb then stops at a terminal outcome, which reads as a broken install
rather than as a new worktree — and `hook pre-tool-use` is worse than that: an
unconfigured repository answers check exit 4 carrying deliberate silence, so the
edit gate allows every write with no output at all, held by
`tests/hooks.rs::unconfigured_no_facts_allows_silently`. A worktree added after
`init` is therefore uncoordinated and says nothing about it.

The pre-edit wrapper is now a presence check plus `exec`, so no front end can add
that diagnostic: it is the engine's to state or nobody's.

Satisfied by: a new worktree of an initialized repository resolving the existing
configuration, or the engine refusing with a diagnostic that names the real cause
and the command that fixes it.

## 4. `reference-transaction` bakes an absolute worktree path into a shared hook

**Target:** `crates/cargo-berth/src/gate/install.rs`.

`.git/hooks/reference-transaction` is shared by every worktree of a repository,
but the installed hook embeds the absolute path of the worktree that installed
it. A second worktree then runs the first one's hook, and removing that first
worktree leaves a hook pointing at a path that no longer exists.

The baked value is the `__POLICY_WORKTREE__` substitution
(`src/gate/install.rs:468`). The same install path owns
`__refresh-managed-hook-after-trunk-deletion`, which carries
`CommandResultReporting::GitHookProtocol` and which no test in this crate invokes
as a command line — its `Cli::run` doc comment says so rather than claiming
coverage it lacks.

Satisfied by: a hook that resolves its own repository and worktree at run time,
with a test that installs from one worktree and exercises the gate from another,
plus a command-line test for `__refresh-managed-hook-after-trunk-deletion`.

## 5. The edit gate honors no bypass at all

**Target:** `~/.claude/scripts/berth/install/hooks/berth_pre_edit.sh` — now a
presence check plus `exec` — and `crates/cargo-berth/src/hook/pre_tool_use.rs`.

`CARGO_BERTH_BYPASS` is read in exactly one place —
`gate::permit::environment_bypass_requested` (`src/gate/permit.rs:166`), called
from `src/cli.rs:1450` on the `reference-transaction` trunk gate — and the
pre-edit wrapper never mentions it. The wrapper checks only whether
`cargo-berth` is on `PATH` and then `exec`s it, so it cannot recover from a hang
at all: after the `exec` there is no shell left to time out. An engine that hangs
or crashes therefore blocks every write with no escape hatch, which is exactly
when one is needed, and the short-circuit has to sit before the `exec`.

Satisfied by: the pre-edit path short-circuiting to an allow on
`CARGO_BERTH_BYPASS=1` before any engine invocation, with the pending-bypass
marker still recorded for the audit, and a test that sets the variable against an
absent or hanging binary and asserts the write proceeds.

## 6. `--integrated-as` requires a prior `release`, unreachable for an orphan

**Target:** `crates/cargo-berth/src/recovery.rs` (`recovery_operation`) and
`crates/cargo-berth/tests/liveness.rs`.

`cargo-berth resolve <id> --integrated-as <commit>` maps an active reservation to
`CheckpointRequired`. An orphaned reservation — one whose worktree is gone —
cannot run `release` from its holder worktree, so the disposition intended to
record rewritten integration cannot be reached.

Satisfied by: a liveness test creating an active orphan and a reachable trunk
commit, then either `--integrated-as` recording rewritten integration directly or
its refusal naming the command that can actually dispose of that orphan.

## 7. Neither `check` nor `claim` refuses a foreign same-worktree reservation

**Target:** `crates/cargo-berth/src/verb/claim.rs`.

A reservation held by a different coordination run in the *same* worktree does
not block a `check` or a `claim` from this run. Two sessions sharing a worktree
can therefore both believe they hold the same paths, which is the condition the
engine exists to prevent.

Phase 7 landed the consolidation: eligibility is now two methods on `Reservation`,
`is_active_for_coordination_run` (`reservation/mod.rs:1867`), which holds the
`Active` lifecycle test, and `is_active_for_coordination_run_and_worktree`
(`:1880`), which adds the worktree term. This item changes what the two-field form
means for `check` and `claim`, so it is unblocked — but the change belongs in the
two-field form or at its call sites, never in the run-only base, whose callers
(`RetainedReservationSet::has_other_active_reservation`, `:1003`) deliberately
reach across worktrees.

Satisfied by: same-worktree foreign reservations entering the refusal path with
their holder facts, and a test covering two runs in one worktree.

## 8. README engine-output quotes are checked against real engine renderings

**Target:** `crates/cargo-berth/README.md` and
`crates/cargo-berth/tests/engine_instructions.rs`.

The README presents engine output as verbatim `text` blocks, but no test ties
those blocks to the Rust renderings that produce them. The scenario machinery now
exists — `tests/engine_instructions.rs` carries named real-binary scenarios
(`POST_TOOL_USE_SCENARIO`, `SESSION_START_SCENARIO`, `run_hook_verb`,
`hook_response_envelope`) — so the remaining work is binding README blocks to
those scenarios rather than building the harness. The drift is recurring, not
hypothetical: the instruction-naming phase required four manual documentation
corrections, the hook-verb phase swept the README by hand again, and the
coordinator cutover carried a third hand sweep over text it changed wholesale.
That cutover also removed the last obstacle — the three installed hooks are
pass-throughs, so every quoted hook block is the engine's own rendering. The one
exception is the wrappers' binary-absent notices, produced without an engine and
asserted directly in `~/.claude/scripts/berth/tests/test_hook_rendering.py`.

Satisfied by: each README block presented as observed engine output being sourced
from a named real-binary scenario or frozen fixture, with a test that fails when
the quoted block differs from that rendering.

## 9. Drift attribution stays ambiguous between two reservations in one worktree

**Target:** `crates/cargo-berth/src/drift/` attribution and
`crates/cargo-berth/tests/liveness.rs`.

When one worktree holds two active reservations, drift cannot attribute a changed
path to either and refuses to widen, printing `DRIFT ATTRIBUTION REQUIRED … run
drift --reservation <id> with one listed reservation`. Because nothing resolves
the ambiguity, the notice fires again on the very next tool call, and it fired on
essentially every call across a full working session. The engine names the fix but
never applies it, so the paths stay unwidened indefinitely.

This is the drift-side companion to item 7: that item is about `check` and `claim`
not refusing a foreign same-worktree reservation, this one is about drift being
unable to choose between two of them.

Satisfied by: drift attributing a changed path to the reservation that already
covers it or that the session identity selects, refusing only when that genuinely
cannot be decided — and, when it does refuse, not repeating the same
unactionable notice on every subsequent invocation.

## 10. A post-commit drift check named an unmodified file as changed

**Target:** `crates/cargo-berth/src/drift/` change detection.

The post-commit drift check reports `Cargo.lock` as a changed path with ambiguous
attribution when that file is neither modified in the working tree nor part of the
commit. It reproduces at every checkpoint commit in this worktree, and `Cargo.lock`
appears only in the post-commit check — the same worktree's pre-edit checks name
only the files actually touched.

Satisfied by: a reproduction that pins what makes an untouched path appear in the
post-commit changed set — a stale index read, a comparison against the wrong tree,
or a generation boundary — and a test covering it.

## 11. The `--reservation` recovery command cannot resolve the refusal that prints it

**Target:** `crates/cargo-berth/src/output.rs` —
`AMBIGUOUS_RESERVATION_RECOVERY_COMMAND` (`:97`) and the explicit-selection
response text (`:3585`).

An ambiguous first touch prints `cargo-berth check --reservation
<reservation-id> <path>...`. Running exactly that widens the scopes and then
answers "The explicit reservation selection applies only to this invocation
because no usable harness session id was supplied; name the reservation again on
a later check" — it publishes no mapping, so the next edit is refused
identically. `CARGO_BERTH_SESSION_ID` is unset in an ordinary Bash tool
environment and only the pre-edit hook ever sets it, per invocation, from the
payload, so the printed instruction is unusable by hand in exactly the situation
that prints it. Reproduced three times during phase 4; prefixing the same
command with `CARGO_BERTH_SESSION_ID=<session uuid>` is what worked.

Satisfied by: the printed recovery command succeeding when run verbatim from a
non-hook environment — proven by a test that runs the rendered command as text
and asserts the following check is no longer ambiguous.

## 12. `board` prints a pointer to its own JSON while holding the rendered report

**Target:** `crates/cargo-berth/src/output.rs` — `render_text` (`:2627`) and
`BOARD_READY_MESSAGE` (`:95`).

`render_text` renders `self.message`, the recovered-bypass markers and the
alerts, and never reads `self.payload.presentation`. So `cargo-berth board`
without `--json` prints "The reservation board was read. Use `cargo-berth board
--json` to inspect it." while the same envelope carries the complete board report
as presentation blocks. Every consumer that reads presentation sees the report;
the one that reads text sees a pointer to it.

Satisfied by: `render_text` rendering the presentation blocks when the envelope
carries them, with a test that runs `cargo-berth board` in a repository holding a
reservation and asserts the reservation appears on stdout.

## 13. Two different conditions print the same summary sentence

**Target:** `crates/cargo-berth/src/output.rs` (`UNSTATED_CONDITION_SUMMARY`,
`:119`) and `crates/cargo-berth/src/hook/post_tool_use.rs`
(`UNAVAILABLE_WORKING_DIRECTORY_SUMMARY`, `:40`).

Both constants are the string "cargo-berth could not inspect this Bash call." One
means the engine answered a condition it does not state in its own words; the
other means the hook's working directory does not exist or is unavailable. A
reader who sees the sentence cannot tell which happened, and the two have
different repairs.

Satisfied by: each condition carrying a summary that names it, with a test
asserting the two texts differ.

## 14. The duplicate-incursion hard stop is asserted nowhere in the crate

**Target:** `crates/cargo-berth/tests/lifecycle.rs` against
`ReservationReplayError::DuplicateIncursionIncident`
(`crates/cargo-berth/src/reservation/mod.rs:2072`).

Journal replay refuses a duplicated incursion record with the status
`duplicate_incursion_incident` and names the command that recovers from it. The
coordinator cutover surfaced the answer — the retired two-call front end had made
the timing cell that reached it unreachable — but nothing under
`crates/cargo-berth/tests/` asserts the status or the rendered text, and the only
named surface is the timing matrix outside the repository.

Satisfied by: a test building a journal with a duplicated incursion record and
asserting the status `duplicate_incursion_incident` and its rendered recovery
command.

## 15. One reporting answer covers two git routes and cannot say which

**Target:** `crates/cargo-berth/src/cli.rs` — `CommandResultReporting::GitHookProtocol`.

`HookProtocol(HookCommand)` names the hook it selects, and the route test asserts
that every harness hook route selects the hook it declares. `GitHookProtocol` is a
unit variant covering both `__reference-transaction` and
`__refresh-managed-hook-after-trunk-deletion`, so the same test can assert only
that the two refuse with `UsageError` — not which route answered. The asymmetry
is why one of the pair has its exit statuses proved end to end in `tests/gate.rs`
while the other has no command-line test in the crate at all.

Satisfied by: `GitHookProtocol` carrying the route it answers for, and the
route test asserting each git route selects the one it declares.

## 16. The published post-tool-use timing bound still describes the retired front end

**Target:** `~/.claude/scripts/berth/tests/installed_front_end.py` —
`POST_TOOL_USE_BOUND_SECONDS` (`:43`).

The bound is `0.20`, measured when the installed hook parsed and validated JSON in
bash and made more than one engine call. The wrappers are now a presence check plus
`exec`, so the number describes a process topology that no longer exists. The
re-key to binary and wrapper availability landed; the measurement did not, because
the cold-page gate requires zero resident pages for `git` and any sibling session
executing git faults them straight back in.

The remaining structural phases are behavior-preserving moves that cannot change
process topology, so nothing later in this plan will make the number wrong in a
new way — but nothing later will fix it either.

Satisfied by: a serialized measurement on a machine with no other active session,
every timing cell from `attribution` onward corrected to its measured process
counts, and the bound republished. Do not widen the bound to make the run green,
and do not loosen `COLD_PAGE_INVALIDATION_ATTEMPTS` or accept a non-zero
resident-page count: both convert a refusal to measure into a false measurement.

## 17. Nothing regression-tests the reservation-id ordering two surfaces now promise

**Target:** `crates/cargo-berth/tests/drift.rs` against
`DriftSelectionError::AmbiguousActiveReservations`
(`crates/cargo-berth/src/drift/selection.rs:254`) and `ResolvedDriftSubjects.reporting`
(`:228`).

Phase 7 routed both through `WireOrderedReservationIds`, so the operator message
`drift is ambiguous; choose one active reservation with --reservation: …` and the
multi-subject reporting list in `drift --json` now print in a stable ascending
order. `reservation_selection_requires_an_explicit_choice_only_when_ambiguous`
(`tests/drift.rs:2020`) asserts only that both identifiers appear, and no test
exercises a multi-element reporting list at all, so the guarantee rests on the
collection type alone. The equivalent surface for `check` is covered —
`tests/overlap.rs:440` compares `candidate_reservation_ids` against a sorted
expectation.

Satisfied by: a drift case with three or more active reservations in one worktree
asserting the ambiguity message names them in ascending rendered order, and a
post-commit case asserting the same for the reporting list.

## 18. An incursion is computed against current holders, so a new claim accuses old commits

**Target:** the incursion detection path in `crates/cargo-berth/` that produces
`Incursion <id>: reservation <id> entered <path> held by foreign reservation(s) <id>`.

Observed 2026-09-01: commit `fd7c9a19` touched `Cargo.lock` on 2026-08-28
12:22:59. A foreign reservation created on 2026-09-01 15:58:50 — four days later
— was reported as the holder that commit "entered", and the incursion record
itself was created at 17:17:02 on a later, unrelated checkpoint that touched no
lockfile. A commit cannot enter a claim that did not exist when it landed.

The comparison must be against the claims in force at the commit's own time, not
against the claims in force at detection time. As written, any new claim over a
long-lived shared file retroactively manufactures an incursion against every
reservation that has ever committed to it, and the operator is told to stop and
resolve an overlap that never happened.

Satisfied by: a test that claims a path, commits to it, releases, then takes a
fresh foreign claim over the same path and asserts no incursion is reported for
the earlier commit; and a second asserting an incursion IS still reported when
the foreign claim predates the commit.

## 19. A proven first-parent interval is carried as a bare map whose missing key means "unproven"

**Target:** `crates/cargo-berth/src/reconcile.rs` —
`PredecessorSuccessorReachability::phase_start_target_histories` (`:604`),
`PendingScopedPatchCandidateContext.target_histories` (`:630`), and its
`candidate` conversion (`:648`).

`git::PhaseStartTargetFirstParentHistories` (`src/git/reachability.rs:106`) already
wraps exactly `HashMap<GitObjectId, Vec<GitObjectId>>` and converts a missing key
into `ScopedPatchTargetHistory::NeedsGitQueries` through `after_phase_start`
(`:110`); phase 9 moved both out of `git/mod.rs`, which is now declarations and
re-exports only.
The reconciliation path lowers the same shape back to a bare map and
re-implements that conversion by hand at `:648`, against
`SuccessorScopedPatchTargetHistory` instead — a second home for a lookup the
crate already names once.

A reader of `:630` has to reach the call site to learn that a missing key means
"no proven interval — query git", not "not computed", and the same empty map is
produced both by `AncestorObjectUnknown` and by a classified head that is
`NotDescendant`. Phase 8's split made the contract visible without changing it;
the module phases are behavior-preserving and cannot own it.

Satisfied by: the pending-candidate context carrying a named type whose absence
case states the meaning — reusing `PhaseStartTargetFirstParentHistories` with a
successor-head accessor, or a sibling that returns
`SuccessorScopedPatchTargetHistory` directly — so `candidate` performs no
`map_or` and no bare `HashMap` crosses a struct field.

Revealed by: Phase 8.
