# cargo-berth — worktree coordination — Next

## Items to consider

- [ ] **Split `drift/` into submodules and move its constants into `constants.rs`**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `drift/mod.rs` is 2,046 lines after Phase 20 and still declares no production submodules. Its constants sit inline where `git/`, `ledger/`, and `worktree/` each use a `constants.rs`; Phase 20 added first-touch post-write attribution to the same module. Style rules 5, 15, 16, 18, 26, 30.
  - Completion condition: `drift/mod.rs` declares production submodules and holds no inline constants; `verify.sh lint cargo-berth` stays green and the drift acceptance tests pass unchanged.
  - Revealed by: Phase 10; evidence corrected after Phases 11, 12, 16, and 20

- [ ] **Rename the internal names that hide their semantic role**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`, `crates/cargo-berth/src/board/mod.rs`, `crates/cargo-berth/src/cli.rs`
  - Why needed: `PriorClassification` does not name its pre-lock foreign-path role; `ReservationRow` names a display representation rather than retained reservation state; and `CommandExecution` does not state who still owns presenting the result. After Phase 17, its `Response` variant is input to `CommandResponseRendering` and may become a normal JSON/text response or a post-commit warning, so the former proposed name `EmitEnvelope` is not truthful. None of these types has a serialized discriminator.
  - Completion condition: rename them to `PreLockForeignPathClassification`, `BoardReservationState`, and `CommandOutputOwnership::{CallerRendersResponse, BoardPresentedAndTerminalRestored}`; serialized payloads remain unchanged, and `verify.sh test cargo-berth`, `verify.sh test cargo-berth board`, `verify.sh test cargo-berth drift`, and `verify.sh lint cargo-berth` stay green.
  - Revealed by: Phase 10; scope corrected after Phases 11–14 and Phase 17

- [ ] **Prove the complete PostToolUse path stays within the published 0.20-second bound**
  - Target: `cargo-berth` drift/reconciliation and the canonical PostToolUse shim at `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh`.
  - Why needed: Phase 17 measured the complete two-reservation PostToolUse call at 0.259 seconds. Phase 20 measured one automatic-widen invocation at 0.180 seconds, but one sample and one outcome do not satisfy the published bound; this cost is paid after every Bash call in an enrolled repository.
  - Completion condition: after Phase 22 updates the canonical shim and registers an exact copy in an enrolled repository, five consecutive complete registered-hook invocations in an enrolled two-reservation repository each finish within 0.20 seconds while preserving the typed clear, ordinary widen, first-touch acquisition, incursion with both `protection.status` states, collision, and attribution outcomes; the Phase 17 shim fixtures, `verify.sh test cargo-berth drift`, and `verify.sh lint cargo-berth` pass.
  - Revealed by: Phase 17; evidence updated after Phase 20

- [ ] **Publish `cargo-berth` after the hana loop is proven and `tui_pane 0.8.0` is published**
  - Target: `cargo-berth` release flow and crates.io publication.
  - Why needed: Phase 15 made the crate publish-ready and its README tells readers to run `cargo install cargo-berth`, but this plan intentionally publishes nothing; the versionless `tui_pane` release pin currently resolves to 0.7.0 while this workspace builds against `tui_pane 0.8.0-dev`.
  - Completion condition: Phase 22 is complete; `tui_pane 0.8.0` is published; `/release cargo-berth 0.1.0` completes its dry run and publish flow; and a fresh `cargo install cargo-berth` succeeds.
  - Revealed by: Phase 15

- [ ] **Make the engine own the status, exit-code, and payload-tag contract consumed by `/sync` and the hook shims**
  - Target: `cargo-berth` output contract, `/Users/natemccoy/.claude/scripts/berth/claim_state.py`, and `/Users/natemccoy/.claude/scripts/berth/install/hooks/{berth_pre_edit.sh,berth_post_bash.sh,berth_session_start.sh}`.
  - Why needed: `claim_state.py` hand-maintains `STATUS_PAYLOAD_KINDS` and `FIXED_STATUS_EXIT_CODES`, while the canonical hook shims separately hand-maintain accepted payload tags and required fields in `jq`. Phase 20 added valid `first_touch`, `first_touch_claimed`, and `post_write_incursion` variants without adding an `OutputStatus`; the Python classifier was updated manually while the canonical hook validators remained stale.
  - Completion condition: one versioned engine-owned contract supplies or mechanically verifies the Python and canonical hook status/exit pairings plus payload tags and required fields; an engine status or serialized enum-variant addition cannot pass engine tests while leaving any front-end consumer stale, and malformed status/payload/exit combinations remain rejected.
  - Revealed by: Phase 18; scope corrected after Phase 20

- [ ] **Expose one named reservation's lifecycle and protected tip through a read-only query**
  - Target: `cargo-berth` engine — `crates/cargo-berth/src/{cli,output}.rs`, `crates/cargo-berth/src/{board,reservation}/mod.rs`, and board integration tests.
  - Why needed: The board deliberately omits lifecycle-bearing rows for a waiting successor and either endpoint of an unresolved overlap. After a lost release reply, `/plan:delegate` can therefore observe `ReservationPresentWithoutProtectedTip` but cannot prove whether that reservation is outstanding or released; a matching retention ref proves only commit reachability.
  - Completion condition: `cargo-berth board --reservation <reservation-id> --json` returns a typed read-only `NamedReservationLifecycle::{Active, Outstanding { protected_tip }, ReleasedAfterCheckpoint { protected_tip, disposition }, ReleasedWithoutCheckpoint { disposition }}` result independent of board placement; an unknown id is a typed invalid-input result rather than `Option`, and waiting-successor plus deferred/blocker fixtures prove the selector while existing board JSON remains compatible.
  - Revealed by: Phase 19
- [ ] **Give the coordinator's classifier surface a semantic return type**
  - Target: `/Users/natemccoy/.claude/scripts/berth/claim_state.py`.
  - Why needed: `classify_claim`, `classify_check`, `render_board`, `_validate_board`, and `_generic_state` all return `dict[str, object]` and reach it through `cast`, so every tagged union the coordinator builds is erased at the one boundary a reader inspects. `ProposalAwaitingApprovalStateValue.proposal` is a bare `dict[str, object]` for the same reason. The weakness predates the coordinator phases — `classify_claim` returned `dict[str, Any]` at `831e34a` — and repairing only one symbol would leave an inconsistent surface.
  - Completion condition: the coordinator's state classifiers return a tagged union rather than `dict[str, object]`, the locked proposal carries a semantic type validated at envelope conversion, no `cast` stands between a tagged value and its return, and basedpyright reports zero errors and zero warnings.
  - Revealed by: Phase 21
