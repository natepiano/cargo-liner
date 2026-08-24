# cargo-berth — worktree coordination — Next

## Items to consider

- [ ] **Split `drift/` into submodules and move its constants into `constants.rs`**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `drift/mod.rs` is 1,781 lines after phase 9b and still declares no production submodules. Its constants sit inline where `git/`, `ledger/`, and `worktree/` each use a `constants.rs`; phase 9b added `session/mod.rs` as another unsplit module-directory root, so drift is no longer the only one. Style rules 5, 15, 16, 18, 26, 30.
  - Completion condition: `drift/mod.rs` declares production submodules and holds no inline constants; `verify.sh lint cargo-berth` stays green and the drift acceptance tests pass unchanged.
  - Revealed by: Phase 9; evidence corrected after Phase 9b

- [ ] **Rename the drift and authorization names that do not state their guarantees**
  - Target: `cargo-berth` crate API — `crates/cargo-berth/src/answer/conflict_authorization.rs`, `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `ConflictAuthorization::Revalidated` names the action taken rather than the guarantee that existing answers cover every current overlap, while `PriorClassification` says only that its data was obtained earlier without naming its pre-lock foreign-path role. A reader has to inspect callers to learn either contract; `EveryActiveForPostCommit` left this scope after phase 9b split reporting from widening, which is exactly what that name now states.
  - Completion condition: each renamed value states its semantic role or guarantee without requiring a caller lookup, and the journal's serialized `kind` tags either stay byte-identical or the rename lands before phase 11 freezes the schema.
  - Revealed by: Phase 9; scope corrected after Phase 9b
