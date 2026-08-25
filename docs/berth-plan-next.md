# cargo-berth — worktree coordination — Next

## Items to consider

- [ ] **Split `drift/` into submodules and move its constants into `constants.rs`**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `drift/mod.rs` is 1,806 lines after phase 16 and still declares no production submodules. Its constants sit inline where `git/`, `ledger/`, and `worktree/` each use a `constants.rs`; phase 11 added `session/mod.rs` as another unsplit module-directory root, so drift is no longer the only one. Style rules 5, 15, 16, 18, 26, 30.
  - Completion condition: `drift/mod.rs` declares production submodules and holds no inline constants; `verify.sh lint cargo-berth` stays green and the drift acceptance tests pass unchanged.
  - Revealed by: Phase 10; evidence corrected after Phase 11, Phase 12, and Phase 16

- [ ] **Rename the internal names that hide their semantic role**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`, `crates/cargo-berth/src/board/mod.rs`, `crates/cargo-berth/src/cli.rs`
  - Why needed: `PriorClassification` does not name its pre-lock foreign-path role; `ReservationRow` names a display representation rather than retained reservation state; and `CommandExecution` does not state who still owns presenting the result. After Phase 17, its `Response` variant is input to `CommandResponseRendering` and may become a normal JSON/text response or a post-commit warning, so the former proposed name `EmitEnvelope` is not truthful. None of these types has a serialized discriminator.
  - Completion condition: rename them to `PreLockForeignPathClassification`, `BoardReservationState`, and `CommandOutputOwnership::{CallerRendersResponse, BoardPresentedAndTerminalRestored}`; serialized payloads remain unchanged, and `verify.sh test cargo-berth`, `verify.sh test cargo-berth board`, `verify.sh test cargo-berth drift`, and `verify.sh lint cargo-berth` stay green.
  - Revealed by: Phase 10; scope corrected after Phases 11–14 and Phase 17

- [ ] **Restore the complete PostToolUse path to the published 0.20-second bound**
  - Target: `cargo-berth` drift/reconciliation and `/Users/natemccoy/rust/hana/.claude/hooks/berth_post_bash.sh`.
  - Why needed: Phase 17 measured the complete two-reservation PostToolUse call at 0.259 seconds, above Phase 15's published 0.20-second bound; this cost is paid after every Bash call in an enrolled repository.
  - Completion condition: five consecutive complete registered-hook invocations in an enrolled two-reservation repository each finish within 0.20 seconds while preserving the typed clear, widen, incursion, collision, and attribution outcomes; the Phase 17 shim fixtures, `verify.sh test cargo-berth drift`, and `verify.sh lint cargo-berth` pass.
  - Revealed by: Phase 17

- [ ] **Publish `cargo-berth` after the hana loop is proven and `tui_pane 0.8.0` is published**
  - Target: `cargo-berth` release flow and crates.io publication.
  - Why needed: Phase 15 made the crate publish-ready and its README tells readers to run `cargo install cargo-berth`, but this plan intentionally publishes nothing; the versionless `tui_pane` release pin currently resolves to 0.7.0 while this workspace builds against `tui_pane 0.8.0-dev`.
  - Completion condition: Phase 21 is complete; `tui_pane 0.8.0` is published; `/release cargo-berth 0.1.0` completes its dry run and publish flow; and a fresh `cargo install cargo-berth` succeeds.
  - Revealed by: Phase 15

- [ ] **Make the engine own the status, exit-code, and payload-kind contract consumed by `/sync`**
  - Target: `cargo-berth` output contract and `/Users/natemccoy/.claude/scripts/berth/claim_state.py`.
  - Why needed: `claim_state.py` hand-maintains `STATUS_PAYLOAD_KINDS` and `FIXED_STATUS_EXIT_CODES` as a mirror of the engine's `OutputStatus`/`BerthExit` pairings; adding a valid engine status currently makes the front end reject that reply until both tables are manually updated.
  - Completion condition: one versioned engine-owned contract supplies or mechanically verifies the Python pairing data, an engine status addition cannot pass engine tests while leaving the front end stale, and malformed status/payload/exit combinations remain rejected.
  - Revealed by: Phase 18

- [ ] **Expose one named reservation's lifecycle and protected tip through a read-only query**
  - Target: `cargo-berth` engine — `crates/cargo-berth/src/{cli,output}.rs`, `crates/cargo-berth/src/{board,reservation}/mod.rs`, and board integration tests.
  - Why needed: The board deliberately omits lifecycle-bearing rows for a waiting successor and either endpoint of an unresolved overlap. After a lost release reply, `/plan:delegate` can therefore observe `ReservationPresentWithoutProtectedTip` but cannot prove whether that reservation is outstanding or released; a matching retention ref proves only commit reachability.
  - Completion condition: `cargo-berth board --reservation <reservation-id> --json` returns a typed read-only `NamedReservationLifecycle::{Active, Outstanding { protected_tip }, ReleasedAfterCheckpoint { protected_tip, disposition }, ReleasedWithoutCheckpoint { disposition }}` result independent of board placement; an unknown id is a typed invalid-input result rather than `Option`, and waiting-successor plus deferred/blocker fixtures prove the selector while existing board JSON remains compatible.
  - Revealed by: Phase 19
