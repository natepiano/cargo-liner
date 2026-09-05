# Refuse a foreign same-worktree reservation — Next

## Items to consider

- [ ] **Record the session-mapping precedence as a precondition of the refusal**
  - Target: `crates/cargo-berth/src/ledger/authorization.rs` `resolve_from_sources`; the as-built occupancy section; a test in `crates/cargo-berth/tests/hooks.rs`
  - Why needed: A live session mapping outranks `CARGO_BERTH_RUN` in `resolve_from_sources`, so a caller inside a mapped session cannot present a second coordination run and the occupancy rule is never asked for it. The refusal depends on this and nothing states it — it is held today only by a test helper's `env_remove`. Note for whoever picks this up: the precedence is a real precondition but it is *not* the cause of the unrefused first touch recorded against this phase; that was reproduced with the session variable unset.
  - Completion condition: The precedence is stated in the as-built occupancy section as a precondition, and a test pins that a `CARGO_BERTH_RUN` supplied alongside a live session mapping is ignored.
  - Revealed by: Phase 1

- [ ] **Thread the acting side's provenance into the occupancy predicate**
  - Target: `Reservation::occupies_worktree_for_another_coordination_run`; `reservation/retention.rs`, `reservation/partition.rs`, `verb/check.rs`, `drift/identity.rs`
  - Why needed: The predicate reads only the holder's coordination-identity provenance; the acting side is guarded separately at each of three call sites by matching an environment-supplied identity first. The rule is symmetric in effect, but the invariant keeping it so is held by a comment rather than by the type, so a fourth call site would apply it to one side only. Unreachable today.
  - Completion condition: Both terms of the rule sit in the predicate, and a new call site cannot apply it one-sidedly.
  - Revealed by: Phase 1

- [ ] **Cover the refusal's ranking against attribution and an unreadable phase start**
  - Target: `crates/cargo-berth/tests/drift.rs`
  - Why needed: The ranking shipped — `OutputStatus::ScopeAcquisitionRefused` now sits above `DriftAttributionRequired` and `ObjectUnknown`, so a refused run is no longer presented as a failed check offering a `drift --full` this rule would refuse. No test pins it. `a_completed_but_refused_run_carries_its_own_status` passes under either ranking because attribution is `NotNeeded` in its fixture, and nothing in the crate produces a drift `PhaseStartObjectUnknown`, which needs a reservation whose `phase_start_head` git cannot resolve.
  - Completion condition: One test drives a refused run whose path attribution is `Ambiguous` or `CoordinationRunRequired`, and one drives a refused run whose phase start is unreadable; each asserts the refusal status rather than the condition it outranks.
  - Revealed by: Phase 1

- [ ] **Record why the post-write first touch need not ask about occupancy**
  - Target: `crates/cargo-berth/src/drift/execution.rs`, the post-write first-touch branch
  - Why needed: That branch takes a reservation and hard-codes scope acquisition as permitted without reaching the locked occupancy question. It is correct only because subject selection matches every active reservation in the worktree regardless of run, so an empty reporting set implies no occupant. As written, an acquisition decision is keyed off a selection fact and the invariant is unrecorded.
  - Completion condition: The invariant is stated beside the branch and pinned by a test, or the branch routes through the same authorization the locked path uses.
  - Revealed by: Phase 1

- [ ] **Name the two-state answer `active_reservation_held_by_another_run` returns**
  - Target: `active_reservation_held_by_another_run` (`crates/cargo-berth/src/reservation/retention.rs`)
  - Why needed: A bare `Option<&Reservation>` where the domain has a named answer. `None` means the worktree is unoccupied for this run and `Some` carries the incumbent with every fact the rejection needs, but the sole caller recovers that only from a `let`-`else` and a variable name.
  - Completion condition: The return type states both answers, and the caller reads them without recovering meaning from a variable name.
  - Revealed by: Phase 1

- [ ] **Make `batched_git_path_distinguishes_spawn_failure_from_completed_failure` load-independent**
  - Target: `crates/cargo-berth/tests/gate.rs:2554`
  - Why needed: The test passes alone in 0.18s and fails under the full parallel suite. After `RawGitBehavior::RemoveAfterTargetHistory` deletes the fake-git wrapper, the engine's next git call under load is `git worktree`, so the envelope is `status: ledger_unreadable` with `git worktree failed: <wrapper>/git: No such file or directory`, while the test asserts the message contains `could not run drift fingerprint`. The assertion pins which git call happens to run first, which load decides. Observed failing 2026-09-04 under the full suite, passing in isolation immediately after; an earlier claim in this session that the fixture git-maintenance fix resolved it was wrong — that fix addressed a different failure mode of a different test.
  - Completion condition: Either the assertion accepts any spawn failure the removed wrapper can produce, or the engine resolves the worktree before the history read so the fingerprint call is deterministically first. Not a widened timeout.
  - Revealed by: Phase 1 final gate
