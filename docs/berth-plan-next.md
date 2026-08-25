# cargo-berth — worktree coordination — Next

## Items to consider

- [ ] **Split `drift/` into submodules and move its constants into `constants.rs`**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `drift/mod.rs` is 1,782 lines after phase 10 and still declares no production submodules. Its constants sit inline where `git/`, `ledger/`, and `worktree/` each use a `constants.rs`; phase 9b added `session/mod.rs` as another unsplit module-directory root, so drift is no longer the only one. Style rules 5, 15, 16, 18, 26, 30.
  - Completion condition: `drift/mod.rs` declares production submodules and holds no inline constants; `verify.sh lint cargo-berth` stays green and the drift acceptance tests pass unchanged.
  - Revealed by: Phase 9; evidence corrected after Phase 9b and Phase 10

- [ ] **Rename the internal names that describe their representation rather than their role**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`, `crates/cargo-berth/src/board/mod.rs`, `crates/cargo-berth/src/cli.rs`
  - Why needed: `PriorClassification` says only that its data was obtained earlier, without naming its pre-lock foreign-path role; `ReservationRow` names a display form rather than the retained reservation's current board state; and `CommandExecution` names the act of running rather than the decision it carries — whether the CLI must emit an envelope, or the board already presented output and restored the terminal so nothing further prints. A reader has to inspect callers to learn any of the three contracts. None carries a serialized discriminator, so phase 11's schema freeze does not constrain when this lands. Phase 10b already renamed the two answer values that do: `ConflictAuthorization::ExistingAnswersCoverEveryOverlap` serializes under `authorization.kind = "existing_answers_cover_every_overlap"`, while `RecordedAnswer::ExistingAnswersCoverEveryOverlap` serializes under `answer = "existing_answers_cover_every_overlap"`.
  - Completion condition: renamed to `PreLockForeignPathClassification`, `BoardReservationState`, and `CommandOutputDisposition::{EmitEnvelope, BoardPresentedAndTerminalRestored}`; no serialized payload changes, and `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` stay green.
  - Revealed by: Phase 9; scope corrected after Phase 9b, Phase 10, Phase 10b, and Phase 10c

- [ ] **Publish `cargo-berth` after the hana loop is proven and `tui_pane 0.8.0` is published**
  - Target: `cargo-berth` release flow and crates.io publication.
  - Why needed: Phase 11 made the crate publish-ready and its README tells readers to run `cargo install cargo-berth`, but this plan intentionally publishes nothing; the versionless `tui_pane` release pin currently resolves to 0.7.0 while this workspace builds against `tui_pane 0.8.0-dev`.
  - Completion condition: Phase 17 is complete; `tui_pane 0.8.0` is published; `/release cargo-berth 0.1.0` completes its dry run and publish flow; and a fresh `cargo install cargo-berth` succeeds.
  - Revealed by: Phase 11
