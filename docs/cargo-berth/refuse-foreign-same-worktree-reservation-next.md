# Refuse a foreign same-worktree reservation — Next

## Items to consider

- [ ] **Record the session-mapping precedence as a precondition of the refusal**
  - Target: `crates/cargo-berth/src/ledger/authorization.rs` `resolve_from_sources`; the as-built occupancy section; a test in `crates/cargo-berth/tests/hooks.rs`
  - Why needed: A live session mapping outranks `CARGO_BERTH_RUN` in `resolve_from_sources`, so a caller inside a mapped session cannot present a second coordination run and the occupancy rule is never asked for it. The refusal depends on this and nothing states it — it is held today only by a test helper's `env_remove`. Note for whoever picks this up: the precedence is a real precondition but it is *not* the cause of the unrefused first touch recorded against this phase; that was reproduced with the session variable unset.
  - Completion condition: The precedence is stated in the as-built occupancy section as a precondition, and a test pins that a `CARGO_BERTH_RUN` supplied alongside a live session mapping is ignored.
  - Revealed by: Phase 1

- [ ] **Give a completed-but-refused drift run its own output status**
  - Target: `OutputStatus`; `crates/cargo-berth/src/output.rs`; `docs/cargo-berth/generated/output-contract.json`
  - Why needed: `OutputStatus::InvalidInput` carries both a request that never ran and a drift run that completed a report and was refused only its scope acquisition. The presented text distinguishes them; the status does not, and `output.rs::refused_scope_acquisition` exists only to re-derive the difference from the payload. A machine reader currently cannot tell an aborted request from a completed one.
  - Completion condition: The completed-but-refused run carries a distinct status and the published output contract is regenerated.
  - Revealed by: Phase 1

- [ ] **Thread the acting side's provenance into the occupancy predicate**
  - Target: `Reservation::occupies_worktree_for_another_coordination_run`; `reservation/retention.rs`, `reservation/partition.rs`, `verb/check.rs`, `drift/identity.rs`
  - Why needed: The predicate reads only the holder's coordination-identity provenance; the acting side is guarded separately at each of three call sites by matching an environment-supplied identity first. The rule is symmetric in effect, but the invariant keeping it so is held by a comment rather than by the type, so a fourth call site would apply it to one side only. Unreachable today.
  - Completion condition: Both terms of the rule sit in the predicate, and a new call site cannot apply it one-sidedly.
  - Revealed by: Phase 1

- [ ] **Keep an unreadable phase start from re-wrapping a refusal as a failed check**
  - Target: `crates/cargo-berth/src/output.rs` drift status selection (the unknown-phase-start branch) and `post_commit_rendering`
  - Why needed: The unknown-phase-start and attribution-required conditions sit above the refusal branch, so a refused run whose phase start cannot be read falls into the post-commit catch-all — the "could not complete the post-commit drift check … run `cargo-berth drift --full` by hand" text, whose by-hand command this same rule would refuse.
  - Completion condition: A refused run with an unreadable phase start is presented as a refusal, not as a failed check with a self-contradicting remedy.
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

- [ ] **Rename `DriftBlockingCoverage::SameIdentity` for the guarantee it now carries**
  - Target: `DriftBlockingCoverage::SameIdentity` (`crates/cargo-berth/src/reservation/partition.rs`)
  - Why needed: The variant now covers a same-worktree holder of another run that has left the active state, and one claimed under an engine-issued identity. Its doc comment has grown several lines whose only job is to say the name is untrue. The guarantee is that the holder has no foreign standing against the subject.
  - Completion condition: The variant name states that guarantee and the doc comment no longer exists to contradict the name.
  - Revealed by: Phase 1

- [ ] **Rename `blocking_coverage_for_drift`'s acting-side parameters**
  - Target: `blocking_coverage_for_drift` parameters (`crates/cargo-berth/src/reservation/retention.rs`)
  - Why needed: Parameters named for the invoking run are fed the subject's actor. Per-subject is the right question, but the current names invite a future caller to pass the invoking run and silently change which holders block.
  - Completion condition: The parameter names say they carry the subject's actor.
  - Revealed by: Phase 1
