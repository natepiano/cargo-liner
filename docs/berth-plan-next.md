# cargo-berth — worktree coordination — Next

## Items to consider

- [ ] **Split `drift/` into submodules and move its constants into `constants.rs`**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`
  - Why needed: `drift/mod.rs` is 2,046 lines after Phase 20 and still declares no production submodules. Its constants sit inline where `git/`, `ledger/`, and `worktree/` each use a `constants.rs`; Phase 20 added first-touch post-write attribution to the same module. Style rules 5, 15, 16, 18, 26, 30.
  - Completion condition: `drift/mod.rs` declares production submodules and holds no inline constants; `verify.sh lint cargo-berth` stays green and the drift acceptance tests pass unchanged.
  - Revealed by: Phase 10; evidence corrected after Phases 11, 12, 16, and 20

- [ ] **Name internal semantic roles and contain external optionality at its boundary**
  - Target: `cargo-berth` crate implementation — `crates/cargo-berth/src/drift/mod.rs`, `crates/cargo-berth/src/board/mod.rs`, `crates/cargo-berth/src/cli.rs`, and `crates/cargo-berth/src/ledger/mod.rs`
  - Why needed: `PriorClassification` does not name its pre-lock foreign-path role; `ReservationRow` names a display representation rather than retained reservation state; and `CommandExecution` does not state who owns presenting the result. In addition, `overlap_authorization_request` exposes six bare `Option<T>` parameters and `EditAuthorization::resolve_from_sources` accepts `Option<OsString>`, so readers must infer overlap-selection and environment-identity states from representation and control flow.
  - Completion condition: rename the three types to `PreLockForeignPathClassification`, `BoardReservationState`, and `CommandOutputOwnership::{CallerRendersResponse, BoardPresentedAndTerminalRestored}`; convert the overlap parser fields into one semantic overlap-selection type before an internal helper receives them; convert the environment lookup immediately into a semantic type distinguishing absent, invalid, and identified coordination-run state; leave bare `Option<T>` only in clap-owned fields and externally required trait signatures; keep serialized payloads unchanged; and keep the cargo-berth test, board, drift, and lint gates green.
  - Revealed by: Phase 10; scope corrected after Phases 11–14, Phase 17, and the Phase 22 type audit

- [ ] **Prove the complete PostToolUse path stays within the published 0.20-second bound**
  - Target: `cargo-berth` drift/reconciliation, the canonical PostToolUse shim at `/Users/natemccoy/.claude/scripts/berth/install/hooks/berth_post_bash.sh`, and its direct registration in `/Users/natemccoy/rust/hana/.claude/settings.local.json`.
  - Why needed: Phase 17 measured the complete two-reservation PostToolUse call at 0.259 seconds. Phase 20 measured one automatic-widen invocation at 0.180 seconds, but one sample and one outcome do not satisfy the published bound; this cost is paid after every Bash call in an enrolled repository.
  - Completion condition: using the directly registered canonical path rather than a repository copy, five consecutive complete live-hook invocations for each of the typed clear, ordinary widen, first-touch acquisition, foreign-only incursion, `post_write_incursion` with `protection.status = acquired`, `post_write_incursion` with `protection.status = not_acquired`, collision, and attribution outcomes each finish within 0.20 seconds; the Phase 17 shim fixtures, `verify.sh test cargo-berth drift`, and `verify.sh lint cargo-berth` pass.
  - Revealed by: Phase 17; evidence updated after Phases 20 and 22

- [ ] **Publish `cargo-berth` after the hana loop is proven and `tui_pane 0.8.0` is published**
  - Target: `cargo-berth` release flow and crates.io publication.
  - Why needed: Phase 15 made the crate publish-ready and its README tells readers to run `cargo install cargo-berth`, but this plan intentionally publishes nothing; the versionless `tui_pane` release pin currently resolves to 0.7.0 while this workspace builds against `tui_pane 0.8.0-dev`.
  - Completion condition: Phase 22's required live Claude Code session has visibly fired all three registered hooks; `tui_pane 0.8.0` is published; `/release cargo-berth 0.1.0` completes its dry run and publish flow; and a fresh `cargo install cargo-berth` succeeds.
  - Revealed by: Phase 15; sequencing corrected after Phase 22

- [ ] **Make the engine own the status, exit-code, and payload-tag contract consumed by `/sync` and the hook shims**
  - Target: `cargo-berth` output contract, `/Users/natemccoy/.claude/scripts/berth/claim_state.py`, and `/Users/natemccoy/.claude/scripts/berth/install/hooks/{berth_pre_edit.sh,berth_post_bash.sh,berth_session_start.sh}`.
  - Why needed: `claim_state.py` hand-maintains `STATUS_PAYLOAD_KINDS` and `FIXED_STATUS_EXIT_CODES`, while the canonical hook shims separately hand-maintain accepted payload tags and required fields in `jq`. Phase 22 had to teach the PostToolUse validator the valid `first_touch_claimed` and `post_write_incursion` variants manually after the Python classifier had already been updated, demonstrating that engine tests do not keep every consumer synchronized.
  - Completion condition: one versioned engine-owned contract supplies or mechanically verifies the Python and canonical hook status/exit pairings plus payload tags and required fields; an engine status or serialized enum-variant addition cannot pass engine tests while leaving any front-end consumer stale, and malformed status/payload/exit combinations remain rejected.
  - Revealed by: Phase 18; scope corrected after Phases 20 and 22

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

- [ ] **Confirm a live Claude Code session fires all three registered hooks**
  - Target: the three canonical shims registered in `/Users/natemccoy/rust/hana/.claude/settings.local.json`.
  - Why needed: a headless session in a throwaway repository with a second worktree has since fired all three shims for real, which settles most of what was unknown: Claude accepts the matcher and command schema, supplies the `cwd` and `session_id` fields the shims read, and resolves `cargo-berth`, `jq`, and `python3` from a PATH-supplied bin directory. PreToolUse refused an edit to a worktree-held path while allowing a free one, PostToolUse reported an auto-widen and an incursion incident after a forced write, and the next SessionStart surfaced that outstanding incursion. What that run did not cover is hana itself: whether Claude reads hana's gitignored `settings.local.json`, and whether the shims resolve their interpreters from the PATH a session rooted there actually has. Wrong-directory execution is especially invisible, because an unconfigured PostToolUse or SessionStart result is deliberately silent.
  - Completion condition: in a session rooted in `hana`, a planted SessionStart notice appears, an edit to a foreign-held path is refused and prevented, an edit to a free path journals a first-touch claim, and a forced Bash write surfaces a PostToolUse incursion.
  - Revealed by: Phase 22

- [ ] **Replace stale-identity retry loops with source-specific typed recovery**
  - Target: `crates/cargo-berth/src/verb/{claim,check}.rs`, `crates/cargo-berth/src/drift/mod.rs`, `crates/cargo-berth/src/output.rs`, `/Users/natemccoy/.claude/scripts/berth/claim_state.py`, and the canonical PreToolUse and PostToolUse shims.
  - Why needed: a stale session mapping or worktree marker survives an ordinary rerun, but `ClaimError::into_output` currently advises only "retry the command"; first-touch `check`, `claim`, and `drift` propagate that result, and PreToolUse can therefore repeat the same refusal indefinitely.
  - Completion condition: stale session mappings and stale marker runs are distinct typed claim/check/drift rejection reasons; the former directs the caller to restart the coordination run or name active work, the latter directs the caller to remove or replace the marker; every canonical consumer renders those recoveries without parsing `message`; and fixtures prove both paths and assert that neither recommends an unqualified rerun.
  - Revealed by: Phase 22

- [ ] **Make the berth install instructions match durable canonical hook registration**
  - Target: `/Users/natemccoy/.claude/scripts/berth/install/README.md` and the hook-registration example it governs.
  - Why needed: `hana` registers the three canonical absolute hook paths directly, while the README instructs consumers to copy the scripts into a repository and remove them afterward; following that procedure permits copies to drift and can leave settings naming deleted files.
  - Completion condition: the README instructs consumers to register the three canonical absolute hook paths directly, explains that each linked worktree needs its own ignored `settings.local.json`, removes the copy-then-delete procedure, and makes uninstallation remove registrations without deleting the canonical scripts; its example matches `hana`'s active settings.
  - Revealed by: Phase 22
