# cargo-berth — worktree coordination — Next

## Items to consider

- [ ] **Split `drift/` into submodules and move its constants into `constants.rs`**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `drift/` is the only module directory in the crate whose root declares no submodule; all nine siblings (`answer`, `edge`, `gate`, `git`, `ledger`, `reservation`, `scope`, `verb`, `worktree`) split. Its constants sit inline where `git/`, `ledger/`, and `worktree/` each use a `constants.rs`. Style rules 5, 15, 16, 18, 26, 30.
  - Completion condition: `drift/mod.rs` declares submodules and holds no inline constants; `verify.sh lint cargo-berth` stays green and the drift acceptance tests pass unchanged.
  - Revealed by: Phase 9

- [ ] **Rename the drift and authorization types that name acquisition rather than guarantee**
  - Target: `cargo-berth` crate API — `crates/cargo-berth/src/answer/conflict_authorization.rs`, `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `ConflictAuthorization::Revalidated` names the action taken, not the guarantee the value carries; `PriorClassification` and `EveryActiveForPostCommit` name how a value was acquired rather than what it means. A reader has to inspect callers to learn the contract.
  - Completion condition: each name states its semantic role or guarantee without requiring a caller lookup, and the journal's serialized `kind` tags either stay byte-identical or the rename lands before phase 11 freezes the schema.
  - Revealed by: Phase 9
