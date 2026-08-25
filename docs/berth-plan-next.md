# cargo-berth — worktree coordination — Next

## Items to consider

- [ ] **Split `drift/` into submodules and move its constants into `constants.rs`**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `drift/mod.rs` is 1,782 lines after phase 10 and still declares no production submodules. Its constants sit inline where `git/`, `ledger/`, and `worktree/` each use a `constants.rs`; phase 9b added `session/mod.rs` as another unsplit module-directory root, so drift is no longer the only one. Style rules 5, 15, 16, 18, 26, 30.
  - Completion condition: `drift/mod.rs` declares production submodules and holds no inline constants; `verify.sh lint cargo-berth` stays green and the drift acceptance tests pass unchanged.
  - Revealed by: Phase 9; evidence corrected after Phase 9b and Phase 10

- [ ] **Rename the internal names that describe their representation rather than their role**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`, `crates/cargo-berth/src/board/mod.rs`
  - Why needed: `PriorClassification` says only that its data was obtained earlier, without naming its pre-lock foreign-path role, and `ReservationRow` names a display form rather than the retained reservation's current board state. A reader has to inspect callers to learn either contract. Neither type carries a serialized discriminator, so phase 11's schema freeze does not constrain when this lands. Phase 10b already renamed the two answer values that do: `ConflictAuthorization::ExistingAnswersCoverEveryOverlap` serializes under `authorization.kind = "existing_answers_cover_every_overlap"`, while `RecordedAnswer::ExistingAnswersCoverEveryOverlap` serializes under `answer = "existing_answers_cover_every_overlap"`.
  - Completion condition: renamed to `PreLockForeignPathClassification` and `BoardReservationState`; no serialized payload changes, and `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` stay green.
  - Revealed by: Phase 9; scope corrected after Phase 9b, Phase 10, and Phase 10b
