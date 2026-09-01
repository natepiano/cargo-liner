# cargo-berth structure and selection fixes

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Removes every
> `clippy::too_many_lines` suppression by splitting the function it guards,
> turns the five module roots that carry logic back into tables of contents,
> repairs the first-touch selection defect that lets replay order override an
> exact session reservation mapping, gives that repair a way out when the
> selection is ambiguous, and moves the coordination logic that lives in the
> installed shell and Python front end into the engine itself — three `hook` verbs
> that decide their own events, guidance that names the engine rather than a
> Python module, and the retirement of the generated second-language validators —
> so an installed front end and an installed binary can no longer disagree and
> installing a new version is the whole repair.

> **As-built disposition: amend** — fold into
> `docs/cargo-berth/as-built/worktree-coordination.md`.

## Delegation Context

- **Project:** `cargo-berth` (workspace member of `cargo-liner`) — a git-worktree
  reservation engine coordinating path ownership and merge order between worktrees.
- **Project started:** 2026-08-30T15:59:59-04:00
- **Stack:** Rust, edition 2024 (workspace), `clap` (derive), `serde`/`serde_json`,
  `schemars`, `crossterm`/`ratatui` (board TUI), `uuid`, `tempfile` (dev).
  **No `lib.rs`** — `main.rs` declares all modules as a binary crate, so
  `cargo nextest run -p cargo-berth --lib` fails; use `--bin cargo-berth`.
- **Layout:** `crates/cargo-berth/src/` — `reservation/`, `verb/`, `edge/`,
  `drift/`, `gate/`, `git/`, `hook/`, `ledger/`, `board/`, `scope/`, `session/`,
  `worktree/`, `answer/`, plus top-level `alert.rs`, `cli.rs`, `config.rs`,
  `constants.rs`, `coordination_identity.rs`, `exit.rs`, `ids.rs`, `output.rs`,
  `output_contract.rs`, `presentation.rs`, `reconcile.rs`, `recovery.rs`.
  `crates/cargo-berth/tests/` holds the integration suites: `answers.rs`,
  `board.rs`, `drift.rs`, `edges.rs`, `gate.rs`, `ledger.rs`, `lifecycle.rs`,
  `liveness.rs`, `overlap.rs`, plus the phase-2 suites `front_end_corpus.rs`,
  `presentation.rs`, and `output_contract.rs`, phase 3's `engine_instructions.rs`,
  phase 4's `hooks.rs`, and the frozen fixture
  `tests/fixtures/front_end_corpus.json`.
- **Front-end and hook layer:** lives outside this repository under
  `~/.claude/scripts/berth/` — `install/install.sh` (installs the binary and
  regenerates `generated/status_payload_tables.py` and
  `generated/envelope_validation.jq` from it, with staging, validation, and
  rollback), the hand-written `install/hooks/berth_pre_edit.sh`,
  `install/hooks/berth_post_bash.sh`, and `install/hooks/berth_session_start.sh`,
  plus `claim_state.py`, `work_order.py`, and `tests/test_hook_rendering.py`. The
  hooks invoke `cargo-berth` from `PATH`, so an installed binary and an installed
  hook can disagree. A phase that changes this layer says so in its summary: the
  checkpoint commit cannot carry files outside the repository.
- **Presentation contract (phase 2, binds phases 3-6):** every verb states its
  own user-facing text as `presentation` on the envelope.
  `crates/cargo-berth/src/presentation.rs` defines `EnvelopePresentation`, a
  `#[serde(tag = "kind", rename_all = "snake_case")]` enum, so the field is never
  absent from real engine output. Phase 4 gave it three variants over the same
  three wire states: `RenderedBlocks { blocks }` carrying a
  `NonEmptyRenderedBlocks` (private field, fallible constructor, so an empty
  rendered-blocks payload is unconstructible), `NothingToShow` for the deliberate
  nothing-to-show case, and `NotProvided`. `NothingToShow` serializes as the
  frozen `{"kind":"rendered_blocks","blocks":[]}` and that object deserializes
  back to it, through a private serde boundary type — two variants under one wire
  tag, which a derived internally-tagged enum cannot express. Every consumer
  prints the rendered text verbatim and classifies nothing.
- **Lints:** the workspace denies `clippy::all`/`cargo`/`nursery`/`pedantic` as
  groups plus per-rule `expect_used`, `panic`, `unwrap_used`, `unreachable`,
  `self_named_module_files`, `undocumented_unsafe_blocks`, and rustc
  `missing_docs` and `unsafe_code`. `too_many_lines` therefore fires from
  `pedantic`; the only conforming answer is a smaller function.
- **Verification:** every phase runs
  `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth` and
  `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`. Run each with the
  sandbox disabled. Tests are the only testing — a passing `test` run proves the
  build, so never add a `check` pass around a `test` that is going to run anyway.
- **Regenerating the output contract:** `output_contract.rs` derives
  `docs/cargo-berth/generated/output-contract.json` from Rust doc comments. Any
  phase that edits a doc comment reachable from the contract must regenerate with
  `CARGO_BERTH_REGENERATE_OUTPUT_CONTRACT=1 cargo nextest run -p cargo-berth
  --bin cargo-berth -E 'test(generated_artifacts_are_reproducible)'` and commit
  the regenerated artifact. Never hand-edit that file.
- **Style:** `~/rust/nate_style/rust/`. Rules this plan is built to satisfy —
  `agent-must-review-allows` (never author an allow or its reason),
  `module-roots-as-table-of-contents`, `when-to-split-a-module`,
  `split-by-type-ownership`, `types-live-with-their-behavior`,
  `tests-live-with-the-type-under-test`, `forbidden-words`.
- **Never run a locally built `cargo-berth` against this repository** — its ledger
  is shared live with two other worktrees. Scratch repositories go under
  `/tmp/claude/` only.

### Constraint that binds every phase

**No phase may add an `#[allow]` or `#[expect]`, or author a `reason` string.**
That is what this plan exists to remove. If a split leaves a lint still firing,
report it rather than suppressing it.

**Module splits are behavior-preserving.** A phase that moves code between files
changes no runtime behavior and adds no test for behavior it did not change. Its
proof is that the existing suite still passes unmodified, plus the visibility
changes the move forces. Never widen an item's visibility beyond `pub(crate)` to
make a move compile; prefer `pub(super)` and re-export through the module root.

---

### Phase 1 — First-touch selection keeps the exact session reservation · status: done

#### As-built

`verb/claim.rs` chooses the reservation a first touch validates against through
`FirstTouchReservationSelection`, a private invocation-local decision type with
four states — `SessionMappedReservation`, `SingleActiveRunReservation`,
`NoActiveRunReservation`, and `AmbiguousActiveRunReservations {
candidate_reservation_ids }`. The choice is made inside the ledger transaction
that already holds the mutation lock, so a successful claim's harness-session
mapping survives the next check and a later widen grows the reservation the
session holds rather than whichever one replay order reaches first.

When no usable mapping selects one active reservation and more than one is
eligible, `check` exits `BlockedByOverlap` with status
`ambiguous_active_run_reservations`, carrying the candidate ids in wire order in
a `first_touch_reservation_selection` payload. That outcome appends nothing,
widens nothing, and publishes no mapping. Eligibility is
`Reservation::is_active_for_coordination_run_and_worktree`: matching coordination
run and worktree, lifecycle `Active`. The widening half lives in its own
`widen_first_touch_reservation` rather than behind a length suppression.

**Files:**
- `crates/cargo-berth/src/verb/claim.rs` — first-touch acquisition, the selection
  type, and the widening path.
- `crates/cargo-berth/src/session/mod.rs` — publishes every `Claim` and `Widen`
  identity into the mapping the selection reads.
- `crates/cargo-berth/src/reservation/mod.rs` — the run-and-worktree eligibility
  predicate.
- `crates/cargo-berth/src/output.rs` — renders the ambiguity outcome and its
  candidates.
- `crates/cargo-berth/src/output_contract.rs`,
  `docs/cargo-berth/generated/output-contract.json` — the status and payload,
  generated rather than hand-written.
- `crates/cargo-berth/tests/overlap.rs`, `tests/answers.rs`, `tests/gate.rs` —
  the acceptance fixtures and the assertions that encoded the earlier behavior.

**Binds later work:** `ambiguous_active_run_reservations` and
`first_touch_reservation_selection` are stable wire names. The eligibility
predicate is an inherent method on `Reservation` and moves with that type. The
ambiguity outcome is user-actionable, has no recovery command, and no installed
front end can classify it — owned by "Recovering from an ambiguous first touch,
and consumers that survive a version change". Reservation-id ordering by rendered
string now has a fifth home here, owned by "One home for run eligibility and
reservation-id ordering".

**Gotchas:** the harness-session mapping is read under the ledger lock but the
acting coordination run is resolved before it; that is safe only because
eligibility requires a matching coordination run, so the unmapped fallback can
only ever select the acting run's own reservation. The mapping is a single-slot
disposable projection — any claim in the same harness session replaces it, and no
command reads a reservation id back from it. `remove_current_session_mapping`
acquires the mutation lock before removing, and that ordering is load-bearing.
The installed hooks invoke `cargo-berth` from `PATH`, so this outcome stays
invisible until the new binary is installed.

**Ruled out:** filtering first-touch eligibility by claim source — the spec
defines eligibility without source, and widening the single active reservation
reduces the ambiguous case rather than causing it. Giving reservation-id ordering
a home in `reservation/mod.rs` — that file becomes a table of contents.

---

### Phase 2 — Recovering from an ambiguous first touch, and consumers that survive a version change · status: done

#### As-built

The engine states every user-facing outcome itself and every consumer prints that
text verbatim. `EnvelopePresentation` is an internally tagged enum
(`#[serde(tag = "kind", rename_all = "snake_case")]`) with two variants,
`RenderedBlocks { Vec<RenderedOutputBlock> }` and `NotProvided`, carrying three
wire states: rendered blocks, an empty-vector `RenderedBlocks` for the deliberate
nothing-to-show case, and `not_provided`. Because the tag is internal, the field
is never absent from real engine output.

`board` emits the complete report as presentation rather than a pointer, through
`BoardModel::envelope_presentation` and `reservation_lifecycle_presentation`.
The front end's classification layer is deleted: the coordinator publishes exactly
one state, `{kind: engine_stated, rendered_markdown}`, and the generated tables
name no status, verb, or payload kind — they export a single
`valid_contract_envelope` shell check. The coordinator retains two agreement
checks, envelope verb against invoked verb and envelope exit code against process
exit; they assert the response belongs to the request and consult no vocabulary.
`generated_python_exports_wire_name_discriminators`, `render_python_tables`, and
`render_jq_validator` are gone, leaving `output_contract.rs` at 393 lines with no
suppression of any kind.

**Files:**
- `crates/cargo-berth/src/presentation.rs` — `EnvelopePresentation`, `RenderedOutputBlock`.
- `crates/cargo-berth/src/board/mod.rs` — `CompleteBoardReport`, `ReservationLifecycleReport`, and both presentations.
- `crates/cargo-berth/src/output.rs` — presentation construction for every verb.
- `crates/cargo-berth/src/output_contract.rs` — the contract builder; schemas and generic consumer artifacts only.
- `crates/cargo-berth/tests/front_end_corpus.rs` + `tests/fixtures/front_end_corpus.json` — a frozen oracle of 50 real engine renderings.
- Outside the repository: `~/.claude/scripts/berth/claim_state.py` (331 lines),
  `generated/status_payload_tables.py` (47 lines), `install/hooks/berth_pre_edit.sh`.

**Binds later work:** the engine's presentation is the only source of user-facing
text, so a phase that adds a payload field renders it as a presentation block; no
consumer reads `payload.data` for display. The `check --reservation` selector is a
third call site of `Reservation::is_active_for_coordination_run_and_worktree`, so
the eligibility-consolidation phase consolidates three sites, not two. The
coordinator's real callers are `~/.claude/commands/sync.md`,
`plan/delegate.md`, and `plan/delegate_checkpoint.md` — not the hooks, which read
presentation directly — so the phase that deletes it migrates those three. The
installed `berth_post_bash.sh` still drives the hidden `--post-tool-use-payload`
round trip, so those compatibility paths survive until the wrapper cutover
installs and deletes them together.

**Gotchas:**
- `EnvelopePresentation` is internally tagged, so `NotProvided` always serializes
  as `{"kind": "not_provided"}`. A fixture omitting the field describes a shape
  the engine cannot emit; a failure from one is a fixture defect.
- The corpus oracle text-compares exactly one entry;
  `EXPECTED_UNCOVERED_CORPUS_ENTRIES = 49` (`tests/front_end_corpus.rs:15`)
  records the rest as deliberately uncovered. It is a frozen oracle, not a
  coverage gate.
- An ignore file under `~/.claude/` hides matches from `rg`. Use `grep -r` when
  sweeping that tree for callers; `rg -l 'claim_state'` returns nothing there
  while three command documents contain it.
- `berth_pre_edit.sh:189` had no fallback for the degraded-success diagnostic and
  now tests `(.kind | nonempty_string)`. The closed checks at `:223` and `:228`
  were left deliberately: their caller publishes identical text either way.
- The proposal token is the serialized proposal, answer included, so a `defer`
  token spent under `--before` re-gates at exit 3 instead of claiming. The
  property is enforced in the engine; the coordinator check for it was redundant.

**Ruled out:**
- A front end that detects version skew and names a repair command — it moves the
  defect rather than removing it; the next unfamiliar status stops the same edits
  for the same reason.
- A generated table of the binary's payload semantics guarded by discriminants —
  attempted and abandoned; one wrong guard reads as too strict and too loose at
  once, and generated fixtures can only confirm the generator agrees with itself.
- Restoring the coordinator's status-to-exit-code agreement check — that is the
  closed vocabulary table this phase deletes.
- Rewriting the two stale-provenance corpus entries — the corpus is a frozen
  oracle of real engine text and those entries record valid renderings.

---

### Phase 3 — Every instruction the engine prints names the engine · status: done

#### As-built

Every shell command the engine renders into a presentation block names
`cargo-berth`. The four reasoned overlap answers print as `cargo-berth claim
<paths...> --before|--after|--defer|--override <holder-reservation-id>
--overlap-why "<reason>"` — no `--cwd`, `PYTHONPATH` or `python3` remains in the
crate — and `drift_message` and `drift_path_attribution_message` carry the binary
name on what were a bare `resolve` and two bare `drift --reservation`.
`blocked_edit_answer_guidance` is a `pub(crate) const fn`, the narrowest widening
that reaches the sibling `cli.rs` test module in a binary crate with no `lib.rs`.
Two guards hold this: `rendered_overlap_answer_commands_select_the_documented_resolution`
reparses each rendered answer through `Cli::try_parse_from` and asserts exactly one
of the four resolutions is selected; `rendered_shell_instructions_invoke_the_engine`
runs the real binary over three scenarios, rejects any recognized shell command in a
rendered block's summary or detail that does not begin with `cargo-berth`, and counts
coverage per scenario so a scenario contributing no command fails.

**Files:**
- `crates/cargo-berth/src/output.rs` — the four overlap answers, `drift_message`, `drift_path_attribution_message`; `blocked_edit_answer_guidance` is `pub(crate)`
- `crates/cargo-berth/src/cli.rs` — the parser-backed unit test
- `crates/cargo-berth/tests/engine_instructions.rs` — real-binary guard over rendered blocks
- `crates/cargo-berth/README.md` — quoted engine output, corrected

**Binds later work:** The README quotes engine output verbatim with nothing binding
the two and no test detecting the drift, so any later change to printed text needs a
manual README sweep. Moving or deleting `blocked_edit_answer_guidance` — as the phase
that retires the coordinator may — moves its parser-backed `cli.rs` test with it. The
block guard's shell vocabulary is `bash`, `cargo`, `git`, `sh`, all rejected inside a
rendered block, so a phase legitimately printing a non-engine instruction fails it with
a message implying the instruction is wrong; the binary-absent wrapper notices from the
phase that retires the coordinator are produced without an engine, fall outside the
guard, and are asserted directly.

**Gotchas:**
- `Cli::try_parse_from` discards `argv[0]`, so a test reparsing a rendered command must check the executable token itself or a rendered `python3 claim …` parses clean and passes.
- `split_whitespace` keeps quote characters, so `--overlap-why ""` reconstructs as the two-character non-empty value `""`; the unit test splits on a three-state shell-quote machine instead.
- A block guard passing when *any* command names the engine lets a bare verb ride beside a good one, and coverage summed across scenarios hides a scenario contributing nothing.
- The block guard matches the rendered executable with `starts_with("cargo-berth")`, not an exact first-token comparison, so a future `cargo-berthx` would pass it.

**Ruled out:**
- Exempting a rendered block from the instruction guard for a later non-engine command — the composite recovery command starts `cd … && cargo-berth …`, which the vocabulary already ignores; extend the guard's coverage instead.

### Phase 4 — `cargo-berth hook pre-tool-use` decides the edit · status: done

#### As-built

- `Command::Hook` is a public verb whose `pre-tool-use` subcommand reads a raw `PreToolUse` payload on stdin and answers the hook's own protocol: nothing on a silent allow, the allow-notice object on stdout when the presentation carries blocks, the refusal detail on stderr with the blocking exit code.
- Every payload part is a domain type with its own absent case, never a bare `Option<T>`: `PreToolUseEditAuthorizationRequest`, `HookWorkingDirectorySelection`, `HarnessSessionIdentityAvailability` (`Available | Unusable`), and a two-type edit-target split — `PayloadEditTarget { Named | NotNamed { reason } }` resolving into `ResolvedEditTarget { WithinRepository | OutsideCoordinationDomain | Unresolved { reason } }`, so `execute()` carries no impossible arm. Optionals live only inside the private serde boundary types.
- `EnvelopePresentation` has three variants, the third being `NothingToShow`; its rendered-blocks payload is `NonEmptyRenderedBlocks` — private field, fallible constructor, so an empty rendered-blocks payload is unconstructible. The board's empty report routes to `NothingToShow`, so the hook's silent-allow path and `ReservationBoardReport::envelope_presentation` read one state. `NothingToShow` serializes as the frozen `{"kind":"rendered_blocks","blocks":[]}` and deserializes back, through a private serde boundary type; that wire object outlives the first-party validators.
- An absent or invalid payload session id publishes a no-session selection that blocks the ambient `CARGO_BERTH_SESSION_ID` fallback. An unresolvable edit path refuses visibly on exit 2 instead of allowing silently. `WorktreeRelativeEditName` rebuilds the worktree-relative name from `Component::Normal` only, so no scope string the hook forms carries a parent component.
- The installed hook layer is untouched; the verb is exercised against it read-only.

**Files:**
- `crates/cargo-berth/src/hook/mod.rs` — hook protocol helpers; PreToolUse-specific except `render_blocks`, `write_allow_notice`, `refuse_hook_request`, `write_stderr_line`.
- `crates/cargo-berth/src/hook/pre_tool_use.rs` — the verb, its payload types, and edit-target resolution; `HookWorkingDirectorySelection` and `HarnessSessionIdentityAvailability` are private here.
- `crates/cargo-berth/src/ledger/mod.rs` — `normalize_absolute_path` and `canonicalize_through_nearest_existing_ancestor`, beside `WorktreeContext`.
- `crates/cargo-berth/src/presentation.rs` — `EnvelopePresentation`, `NonEmptyRenderedBlocks`.
- `crates/cargo-berth/src/session/mod.rs` — `OnceLock<HookHarnessSessionSelection>`.
- `crates/cargo-berth/src/cli.rs` — the `Hook` subcommand, `CommandResultReporting`.
- `crates/cargo-berth/tests/hooks.rs` — 20 tests.

**Binds later work:** both path helpers live in `ledger/mod.rs` and are reused from there under the soundness rule below; `normalize_absolute_path`'s doc comment states that rule and must travel intact if the helper moves. The session-start verb must publish the same no-session selection this verb publishes, or an absent payload identity picks up an ambient `CARGO_BERTH_SESSION_ID` again. `EnvelopePresentation` has three variants, not two. The `HookProtocol` answer is the exception to envelope/exit agreement. Every installed-wrapper edit belongs to the wrapper cutover.

**Gotchas:**
- `normalize_absolute_path` collapses `..` textually; POSIX resolves `..` after symlinks. The collapse is sound only when every component left of a `..` is a real directory — a payload-named edit target goes to `canonicalize_through_nearest_existing_ancestor` uncollapsed. Reversed, the hook coordinates a file the write never touches while the write lands outside the repository uncoordinated.
- `Path::file_name()` is `None` for a path ending in `..`, so `<repo>/absent/../held.rs` reaches `NoExistingAncestor` and refuses visibly rather than resolving.
- A linked git worktree does not inherit `.claude/config/berth.toml`, so an unenrolled requester answers exit 0 for every edit.
- `hook/mod.rs` is not a shared event protocol; most of it is PreToolUse-specific.
- The refusal names a recovery command that cannot resolve it: an explicit `--reservation` selection does not persist without `CARGO_BERTH_SESSION_ID`, and nothing but the hook sets it.

**Ruled out:**
- A refusal prefix on any arm of `render_refusal` — all four blocking arms self-frame, so a blanket prefix double-frames.
- Widening `assert_no_parent_component_was_claimed` to scan stdout — unreachable by construction until the helper is reused on an allow-with-notice arm.

### Phase 5 — `cargo-berth hook post-tool-use` and `hook session-start` · status: todo

#### Work Order

**Goal:** The two remaining hook events are decided inside the engine.

**Spec:** Two verbs.

`hook post-tool-use` is the closest to done of the three. The engine already
parses the raw payload (`PostToolUseDriftInvocation::from_value`,
`crates/cargo-berth/src/cli.rs:1214`, its struct at `:1180`) and already emits
the exact hook response
object — `continue`, `systemMessage`, and `hookSpecificOutput` with
`hookEventName` and `additionalContext` — in `emit_post_tool_use_rendering`
(`crates/cargo-berth/src/cli.rs:2008`). What lives in the shell is the
orchestration: `PostToolUseRendering::RequiresLiveIncursionBoard` currently
returns the drift envelope to `berth_post_bash.sh`, which feeds it back to a
second invocation as `PostToolUseLiveIncursionBoardInput`. Move that round trip
inside the verb so one process performs drift, decides whether a live incursion
board is required, assembles it, and emits one response.

That decision needs a name. `PostToolUseRendering::RequiresLiveIncursionBoard`
(`crates/cargo-berth/src/output.rs:331`) is today a wire-return marker meaning
"hand this back to the shell"; once the round trip is internal it is a domain
decision about what the response must carry, and leaving it as a marker variant
read by one `match` misnames it.

`hook session-start` reads the `SessionStart` payload, runs the board, and emits
the session response carrying the engine's actionable presentation.
`berth_session_start.sh`'s `session_notices`, `engine_stated_board_message` and
`render_replay_failure_route` describe what it must produce.

The board's complete report is **already done**: phase 2 gave
`BoardModel::envelope_presentation` a `rendered_blocks` presentation carrying the
serialized complete report for a populated board, held by
`tests/board.rs::populated_board_presentation_carries_the_complete_board_report`.
`hook session-start` reads that presentation; it does not add one. Do not add a
machine-readable action list either: no consumer needs one once the hooks are
verbs, and **Delegation Context** forbids opening surface without a consumer.

**Files:**
- `crates/cargo-berth/src/hook/mod.rs`
- `crates/cargo-berth/src/hook/pre_tool_use.rs`
- `crates/cargo-berth/src/hook/post_tool_use.rs`
- `crates/cargo-berth/src/hook/session_start.rs`
- `crates/cargo-berth/src/cli.rs`
- `crates/cargo-berth/tests/hooks.rs`
- `crates/cargo-berth/tests/engine_instructions.rs`
- `crates/cargo-berth/tests/front_end_corpus.rs`
- `crates/cargo-berth/README.md`

**Seats:** 2 writers + 1 tester — the two verbs are independent files over a
protocol module phase 4 left PreToolUse-specific, and the tests are their own lane.
- `impl` — `cli.rs`, `hook/mod.rs`, `hook/pre_tool_use.rs`, and
  `hook/post_tool_use.rs`; hub: `hook/mod.rs` (where phase 4's PreToolUse-specific
  protocol is generalized and the shared payload types land)
- `test` — `tests/hooks.rs`, `tests/engine_instructions.rs`, and
  `tests/front_end_corpus.rs`
- `review` — opens as `impl`; `hook/session_start.rs` and `README.md`; hub:
  `hook/session_start.rs`

**Acceptance gate:**
1. `cargo-berth hook post-tool-use` performs drift and any required live
   incursion board in one process and emits the same response object and exit
   code the installed hook produces today, including the no-feedback exit.
2. `cargo-berth hook session-start` emits the same session response the installed
   hook produces today, with one deliberate difference the test states
   explicitly: an absent or invalid payload session id publishes a no-session
   selection and cannot fall through to an ambient `CARGO_BERTH_SESSION_ID`. A
   test asserts that an ambient value set in the environment does not reach the
   verb's identity selection.
3. Every `PostToolUse` and `SessionStart` case in
   `tests/fixtures/front_end_corpus.json` — feedback and silence alike — is
   covered by a test driving the raw hook payload through the new verb and
   comparing user-visible output. The fixture's `payload.data` classifiers are
   not ported. Phase 4 established the mechanism: add entries to
   `HOOK_ACCEPTANCE_TEXT_COMPARED_ENTRIES` and lower
   `EXPECTED_UNCOVERED_CORPUS_ENTRIES` (`tests/front_end_corpus.rs:15`, now 45)
   to match, which is why that file is in **Files** and in the test lane.
4. A test drives a raw `PostToolUse` payload and a raw `SessionStart` payload —
   not a constructed inner type — through their verbs and asserts the drift and
   resolve instructions the user is told to run.
5. Every quoted engine-output block and hook description in
   `crates/cargo-berth/README.md` still matches what this phase's code emits.
6. `tests/engine_instructions.rs` covers the rendered blocks this phase adds, and
   its guard passes over them unchanged.
7. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 4 established the `Hook` verb, its
stdin payload types and the `hook/` module layout — extend them rather than
opening a parallel shape. Two facts about what it actually left, both verified
against the tree:

- `hook/mod.rs` is **not** a shared event protocol yet. `PreToolUseAllowNotice`,
  `PreToolUseAllowNoticeDetail`, `render_pre_tool_use_answer`,
  `AUTHORIZED_SYSTEM_MESSAGE` and `FAIL_OPEN_SYSTEM_MESSAGE` are all
  PreToolUse-specific; only `render_blocks`, `write_allow_notice`,
  `refuse_hook_request` and `write_stderr_line` generalize. Generalizing the rest
  is this phase's work, not a precondition it inherits.
- `HookWorkingDirectorySelection` and `HarnessSessionIdentityAvailability` are
  **private** to `hook/pre_tool_use.rs` — no `pub(super)`. Reusing them means
  moving them to `hook/mod.rs`, which is why `hook/pre_tool_use.rs` is in this
  phase's **Files** and owned by `impl`.

`normalize_absolute_path` and `canonicalize_through_nearest_existing_ancestor`
now live in `crates/cargo-berth/src/ledger/mod.rs`. This phase's only use is
`normalize_absolute_path` on a payload `cwd`, which is the sound case — a
directory the harness reports itself sitting in. The rule that makes it sound:
the collapse is textual, so it holds only when every component left of a `..` is
a real directory; anything an edit payload names as a target goes to
`canonicalize_through_nearest_existing_ancestor` uncollapsed instead. Phase 4
shipped a defect from getting this backwards.

Phase 4 also changed `BoardModel::envelope_presentation`
(`crates/cargo-berth/src/board/mod.rs:652-660`) to route its empty case to
`EnvelopePresentation::NothingToShow` rather than an empty vector. That is
`hook session-start`'s silence path: the board presentation it reads now has
three states to branch on, not two. The two-step live-incursion round trip exists because
the shell was the only place that could hold intermediate state, and it has no
reason to survive inside one process. **But the installed `berth_post_bash.sh`
still drives it** — `cargo-berth drift --json --post-tool-use-payload` at `:431`
and `cargo-berth board --json --post-tool-use-payload` at `:450`. Move the round
trip inside the verb and leave the `--post-tool-use-payload` plumbing in place;
deleting it here would break the installed hook before its wrapper exists. Phase
6 installs the wrappers and deletes that plumbing in one atomic cutover.

**This phase does not touch the installed hook layer at all.** Like phase 4, it
builds and tests engine verbs against the current hooks read-only; nothing here
lands outside the repository, so the checkpoint carries the whole phase.

Name the session-start payload's parts the way phase 4 named the pre-tool-use
payload: a `SessionStartReconciliationRequest` for the request itself, reusing
phase 4's `HookWorkingDirectorySelection` rather than introducing a second
working-directory type.

A second one already exists and this phase owns its verb:
`PostToolUseWorkingDirectory::{CurrentProcess, Supplied(PathBuf)}`
(`crates/cargo-berth/src/cli.rs:1174`). Moving post-tool-use into
`hook/post_tool_use.rs` without collapsing it into `HookWorkingDirectorySelection`
ships exactly the duplication the sentence above exists to prevent, so collapse
it as part of the move.

**The harness session identity fails closed here too.** Phase 4 found that an
absent or invalid payload session id fell through to the ambient
`CARGO_BERTH_SESSION_ID`, and closed that by having the hook publish a no-session
selection that blocks the fallback. `berth_session_start.sh` reads no `session_id`
and sets no `CARGO_BERTH_SESSION_ID` at all, so a session-start verb that
publishes a no-session selection is a **deliberate behavior change**, not parity
with the installed hook — gate item 2 is written accordingly. Make the change:
an engine verb that can silently borrow another session's identity is the defect
phase 4 closed for one event, and leaving it open for another would reintroduce
it by a different door.

**This phase owns the drift and resolve instruction surface.** The commands a
user is told to run after an edit (`PostToolUse`) or at session start
(`SessionStart`) are this phase's user-actionable consequence, and gate item 4 is
its acceptance test.

Phase 3's two guards bind here as they do in phase 4:
`tests/engine_instructions.rs` rejects any rendered-block shell command whose
executable is not `cargo-berth`, and the parser-backed `cli.rs` test feeds each
rendered answer command back through the real command-line surface. Phase 3 also
proved stale README quotes go undetected by the suite, so any change to printed
text requires a sweep of every quoted block in `crates/cargo-berth/README.md`.

---

### Phase 6 — Retire the coordinator and the generated validators · status: todo

#### Work Order

**Goal:** The installed front end is three thin hook wrappers and an installer.
`claim_state.py`, the generated Python status tables, and the generated jq
validator are gone; every caller that reached the coordinator reaches the binary
instead; and the behavior they covered is covered by Rust tests.

**Spec:** Phase 2 already deleted the classification layer and the large
generated tables. What is left to retire is smaller and different from what this
Work Order originally described, so take this inventory as the scope:

| Residual | Size | Disposition |
| --- | --- | --- |
| `~/.claude/scripts/berth/claim_state.py` | 331 lines | deleted, after its callers migrate |
| `~/.claude/scripts/berth/generated/status_payload_tables.py` | 47 lines | deleted |
| `~/.claude/scripts/berth/generated/envelope_validation.jq` | 34 lines | deleted |
| `install/hooks/berth_pre_edit.sh` | 597 lines | reduced to a wrapper |
| `install/hooks/berth_post_bash.sh` | 570 lines | reduced to a wrapper |
| `install/hooks/berth_session_start.sh` | 378 lines | reduced to a wrapper |
| `install/install.sh` | — | loses generated-artifact staging, validation, rollback |
| `output_contract.rs` artifact constants and builders | — | lose the two consumer artifacts |

The three generator functions this Work Order used to name —
`render_python_tables`, `render_jq_validator`, and
`generated_python_exports_wire_name_discriminators` — **no longer exist**;
`output_contract.rs` is 393 lines and carries no suppression at all. What remains
there is the artifact constants and the builder entries that publish
`consumer_artifacts`. Remove only the two consumer artifacts and their builders;
the contract's schemas stay.

**The coordinator has callers outside the hooks, and the original gate could not
see them.** The three hooks read envelope presentation directly and do not
invoke `claim_state.py` at all. Its real consumers are command documents:
`~/.claude/commands/sync.md`, `~/.claude/commands/plan/delegate.md`, and
`~/.claude/commands/plan/delegate_checkpoint.md`. Deleting the module without
migrating them leaves three commands calling something that is not there. Each
must move to invoking `cargo-berth` directly while preserving what the
coordinator did for them: repository-root resolution from any directory inside
the repository, `CARGO_BERTH_SESSION_ID` propagation from
`CLAUDE_CODE_SESSION_ID`, one invocation per command with no retry, access to the
proposal token and envelope fields, and a `state.rendered_markdown` equivalent to
print verbatim. Rewrite the command documents in the same phase that deletes the
module.

Reduce each hook to a binary presence check plus `exec cargo-berth hook <event>`,
keeping each hook's existing failure mode when the binary is absent: fail closed
for pre-edit, a static repair notice for post-bash, a static installation notice
for session-start. That failure mode is the one piece of policy that must stay
outside, because it is what the front end says when there is no engine to ask.

**This phase is the atomic cutover.** Phases 4 and 5 deliberately left the hidden
`--post-tool-use-payload` paths in place because the installed `berth_post_bash.sh`
still drives them. Installing the wrappers and deleting those paths happens here,
together, or the installed hook breaks between phases.

`tests/front_end_corpus.rs` runs both generated validators — it extracts
`consumer_artifacts` from the contract and calls `run_python_shell_consumer` and
`run_jq_shell_consumer`, failing if either artifact is absent. Delete that
validator-compatibility lane and keep the frozen text oracle: the fixture
`tests/fixtures/front_end_corpus.json` records real engine renderings and stays.

`tests/test_hook_rendering.py` keeps only what genuinely remains outside — that
each installed wrapper execs the binary, and that each hook's binary-absent
failure mode holds. Do not delete a behavior assertion without naming its Rust
replacement. `HookTimingTests` **cannot simply stay**: it fingerprints both
generated validators, requires the jq validator to exist, measures a
`generated_validator_needs_repair` outcome that this phase removes, and borrows
infrastructure from `HookRenderingTests`. Re-key its matrix to binary and wrapper
availability, extract the infrastructure it still needs into its own module, and
remeasure the published timing bound — the process topology changes, so the old
number no longer describes anything.

`work_order.py` is not berth runtime — it validates plan documents and does not
touch the ledger. Leave it exactly where it is.

The engine keeps a documented envelope shape for any independent consumer that
appears later. A second-language validator on the first-party path is what let an
installed front end and an installed binary disagree, and that is the defect this
plan exists to close.

**Files:**
- `crates/cargo-berth/src/cli.rs`
- `crates/cargo-berth/src/output.rs`
- `crates/cargo-berth/src/output_contract.rs`
- `crates/cargo-berth/tests/front_end_corpus.rs`
- `crates/cargo-berth/tests/hooks.rs`
- `crates/cargo-berth/README.md`
- `docs/cargo-berth/generated/output-contract.json`

**Seats:** 2 writers + 1 tester — the in-repository deletions and the
outside-the-repository front end are disjoint, and the Rust and Python test
lanes are a third group.
- `impl` — `cli.rs`, `output.rs`, `output_contract.rs`, and the generated output
  contract; hub: `cli.rs` (where the legacy dispatch is removed)
- `test` — the Rust hook, corpus, and instruction tests, and
  `tests/test_hook_rendering.py`
- `review` — opens as `impl`; the installed hooks, the installer, the
  coordinator, the command documents, and `README.md`; hub: `install/install.sh`

**Acceptance gate:**
1. `grep -rl 'claim_state' ~/.claude/` returns nothing, and both generated
   artifacts are gone along with the builders that emitted them. Use `grep -r`,
   not `rg`: an ignore file under `~/.claude/` silently hides these matches
   from `rg`, which is how the callers above went unnoticed.
2. `/sync`, `/plan:delegate`, and `/plan:delegate_checkpoint` invoke
   `cargo-berth` directly and still resolve the repository root from a
   subdirectory, propagate the harness session id, invoke once, and print the
   engine's rendered text verbatim.
3. Each hook is a wrapper that execs the binary, and a test proves each one's
   binary-absent failure mode still holds.
4. A Rust CLI test proves what the coordinator's verb and exit-code agreement
   checks proved, with the exception phase 4 introduced: every command whose
   `Command::result_reporting()` answers `CommandResultReporting::Envelope(verb)`
   emits that verb in the envelope and returns the envelope's exit code as its
   process exit. A command answering `CommandResultReporting::HookProtocol` emits
   no envelope and owns its own exit code through
   `CommandOutputOwnership::HookRendered(ExitCode)`; the test asserts that
   exception holds rather than skipping those commands.
5. Every wrapper and binary-absence case in
   `tests/fixtures/front_end_corpus.json` is covered by a Rust or wrapper test;
   the validator-compatibility lane is removed and the frozen text oracle is
   retained.
6. `HookTimingTests` is re-keyed to binary/wrapper availability, names no
   generated artifact, and publishes a remeasured bound.
7. Every behavior assertion removed from `test_hook_rendering.py` is named
   alongside the Rust test that now covers it.
8. The hidden `--post-tool-use-payload` flags,
   `PostToolUseLiveIncursionBoardInput` in `src/cli.rs`, and
   `PostToolUseRendering::RequiresLiveIncursionBoard` in `src/output.rs` are all
   gone, and `blocked_edit_answer_guidance` either stays put or moves with its
   parser-backed `cli.rs` test.
9. Every quoted engine-output block and hook description in
   `crates/cargo-berth/README.md` still matches what the code emits after the
   cutover.
10. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass, and
    the surviving Python suite passes.

**Constraints from prior phases:** phases 4 and 5 must both have landed and been
installed — deleting the coordinator before all three verbs work leaves three
worktrees with no edit gate. Install and exercise the new wrappers against a
scratch repository under `/tmp/claude/` before deleting anything, and keep the
existing rollback copies until the installed path is green. **The whole of this
phase's front-end half lives outside the repository and cannot be committed —
say so in the summary.**

**This phase owns every wrapper edit and installation in the plan.** Phases 4 and
5 deliberately left the installed hooks untouched, so nothing is half-cut over
when this phase starts.

The compatibility plumbing this phase deletes lives in two source files the
original Files list omitted: the hidden `--post-tool-use-payload` flags and
`PostToolUseLiveIncursionBoardInput` are in `src/cli.rs`, and
`PostToolUseRendering::RequiresLiveIncursionBoard` is in `src/output.rs`. Phase 3
added a parser-backed test in `src/cli.rs` that feeds each rendered
overlap-answer command back through the real command-line surface; if
`blocked_edit_answer_guidance` moves, that test moves with it rather than being
dropped.

**This phase owns byte-for-byte wrapper pass-through.** Each reduced wrapper must
deliver the engine's `PreToolUse`, `PostToolUse`, and `SessionStart` output
unchanged — that is the user-actionable consequence of the cutover, and gate
items 3 and 5 are its acceptance tests. The wrappers' binary-absent notices are
the one text produced without an engine, so `tests/engine_instructions.rs` does
not govern them; assert them directly instead.

Phase 3 proved stale README quotes go undetected by the suite. This phase changes
what every hook prints, so sweep every quoted block and hook description in
`crates/cargo-berth/README.md`.

---

### Phase 7 — One home for run eligibility and reservation-id ordering · status: todo

#### Work Order

**Goal:** The active-for-this-run predicate and the deterministic ordering of
reservation ids each have exactly one implementation, placed so the later module
phases move code without carrying a duplicate with them.

**Spec:** Two idioms are spread across the crate, and the module phases that
follow cannot consolidate them because neither belongs to a single module root.

The eligibility predicate exists as
`Reservation::is_active_for_coordination_run_and_worktree`
(`crates/cargo-berth/src/reservation/mod.rs:1855`), which phase 1 added and
`verb/claim.rs:394`, `:422`, and `:442` call — `:453` is the reservation-id sort
the next paragraph owns, not a fourth eligibility call — while the same `actor.run == …
&& actor.worktree == …` comparison is still written out inline at
`reservation/mod.rs:826`, `:839`, and `:1006`. Route every site that means
"active for this run and worktree" through the method. Where a site means
something narrower, say so at that site rather than widening the method.

Reservation-id ordering by rendered string appears five times:
`verb/claim.rs:453` (`sort_by_cached_key`), `drift/ordering.rs:12`,
`output.rs:3677`, `board/mod.rs:945`, and `gate/mod.rs:962`. `drift::ordering` is
`pub(super)` to `drift`, so no other caller can reach it. Give the ordering one
home with `ReservationId` in `crates/cargo-berth/src/ids.rs`, and encode the
guarantee in the type rather than in a comment: a `Vec<ReservationId>` that four
call sites promise to have sorted is not a guarantee, and phase 1's candidate
list documents its determinism only in prose. Introduce a named ordered
collection — `WireOrderedReservationIds` or an equally explicit name — that can
only be constructed sorted, and have the wire-facing producers hold it.

This is behavior-preserving. Every existing test passes unmodified, and the
ordering the wire already emits does not change.

**Files:**
- `crates/cargo-berth/src/ids.rs`
- `crates/cargo-berth/src/reservation/mod.rs`
- `crates/cargo-berth/src/verb/claim.rs`
- `crates/cargo-berth/src/drift/ordering.rs`
- `crates/cargo-berth/src/drift/selection.rs`
- `crates/cargo-berth/src/output.rs`
- `crates/cargo-berth/src/board/mod.rs`
- `crates/cargo-berth/src/gate/mod.rs`

**Seats:** 2 writers + 0 testers + reserve — the ordering type and the
eligibility consolidation are independent, and the phase changes no behavior, so
there is no lane to write tests against ahead of the code.
- `impl` — `ids.rs` and every ordering caller; hub: `ids.rs`
  (`WireOrderedReservationIds` and its unit test)
- `test` — opens as `impl`; the `reservation/mod.rs` eligibility consolidation
- `review` — reserve

**Acceptance gate:**
1. A crate-wide sweep finds one implementation of the run-and-worktree
   eligibility predicate and one of the reservation-id ordering, with no inline
   restatement of either.
2. The ordered collection cannot be constructed unsorted, and that is proven by
   a test rather than asserted in a comment.
3. The existing suite passes unmodified — this phase changes no behavior.
4. `generated_artifacts_are_reproducible_from_the_checked_in_contract` passes
   without the regenerate environment variable, proving the wire did not move.
5. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 2 **did** add a third call to the
eligibility method — the `check --reservation` selector's explicit reservation
selection (`crates/cargo-berth/src/verb/claim.rs:365-368`) — so this phase
consolidates three call sites, not two, and that selector uses the single home
this phase establishes. This consolidation runs before the module phases deliberately:
the ordering must not land in `reservation/mod.rs`, whose own phase reduces it to
a table of contents, and the predicate stays with the `Reservation` type so it
moves with that type when that phase runs.

---

### Phase 8 — Split the reconciliation planners · status: todo

#### Work Order

**Goal:** `reconcile.rs` carries no `too_many_lines` suppression.

**Spec:** `build_plan` (`:866`, 127 lines) and `successor_incorporation_evidence`
(`:1683`, 299 lines) are the two sites. `successor_incorporation_evidence` walks
predecessor subjects, evaluates scoped patch equivalence under the shared
per-reconciliation budget, and assembles verdicts; each of those is a separate
function on the same data. Extract them so the parent states the sequence and
each step owns its own reasoning. `build_plan` splits along the same
boundary its comments already name.

Note the budget type is now the enum
`ReconciliationSuccessorScopedPatchEvaluationBudget::{Unspent, Spent}` with
`evaluate(&mut self, impl FnOnce() -> ScopedPatchComparison)`; a split must keep
the single-admission guarantee, which means the budget stays threaded through
one owner rather than copied into each extracted function.

**Files:**
- `crates/cargo-berth/src/reconcile.rs`

**Seats:** 1 writer + 1 tester + reserve — nothing splits. The whole phase is one
file, and the single-admission budget must stay threaded through one owner, so a
second writer would contend on every extraction.
- `impl` — `reconcile.rs`; hub: `reconcile.rs`
- `test` — verification only, against the existing reconciliation lane; owns no
  source
- `review` — reserve

**Acceptance gate:**
1. No `too_many_lines` suppression remains in `reconcile.rs`.
2. The existing reconciliation tests pass unmodified — this phase changes no
   behavior.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** none — no earlier phase touched this file.

---

### Phase 9 — `git/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `git/mod.rs` declares submodules and re-exports, and carries no logic
and no `too_many_lines` suppression.

**Spec:** The root holds roughly 3,100 lines past its declarations — the largest
offender in the crate — beside existing `command.rs`, `constants.rs`, and
`refs.rs` submodules. Split by type ownership, not by code category: each new
submodule is named after the anchor type or the git concept it owns, and its
tests move with it. Candidate boundaries visible today, to be confirmed against the
code rather than taken as given: reachability and ancestry queries (including
`commit_target_reachability` at `:1884`, 151 lines, which is also a
`too_many_lines` site and must be split rather than moved intact); scoped patch
comparison and the symmetric-difference reader that
`ProtectedUnmatchedCommit` now types; merge-conflict coverage classification
(`ScopedMergeConflictCoverage`); branch and retention-ref updates; worktree and
administrative-directory discovery.

The root keeps only `mod` declarations followed by re-exports. Anything a sibling
module needs becomes `pub(super)` in its new home and is re-exported from the
root under the name callers already use, so no call site outside `git/` changes.
Name each new submodule explicitly in the summary; the Files list below carries
the ones the split is expected to create.

**Files:**
- `crates/cargo-berth/src/git/mod.rs`
- `crates/cargo-berth/src/git/reachability.rs`
- `crates/cargo-berth/src/git/patch.rs`
- `crates/cargo-berth/src/git/conflict.rs`
- `crates/cargo-berth/src/git/refs.rs`
- `crates/cargo-berth/src/git/discovery.rs`

**Seats:** 3 writers + 0 testers — the split is by type ownership and the
clusters do not appear in each other's field lists, so three writers can carve
disjoint submodules out of one root. Tests move with the types, so there is no
separate test lane.
- `impl` — `git/mod.rs` and `reachability.rs`; hub: `git/mod.rs` (the root every
  writer re-exports through)
- `test` — opens as `impl`; `patch.rs` and `conflict.rs`
- `review` — opens as `impl`; `refs.rs` and `discovery.rs`

**Acceptance gate:**
1. `git/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. No `too_many_lines` suppression remains anywhere under `crates/cargo-berth/src/git/`.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** none — no earlier phase added a call into
`git/`. The submodule names in **Files** are the expected split; if the code
argues for a different boundary, take it and say so, but every new file must be
named in the summary.

---

### Phase 10 — `reservation/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `reservation/mod.rs` declares submodules and re-exports, and carries no
`too_many_lines` suppression.

**Spec:** Roughly 2,500 lines past its declarations, beside existing
`constants.rs`, `evidence.rs`, and `lifecycle.rs`. Two suppressions live here:
`apply` (`:1015`, 150 lines) and a `Display::fmt` (`:2094`, 103 lines). The
`fmt` is an exhaustive match over a large enum — split it by giving each variant
family its own renderer, keeping the outer match as the dispatch.

Split by type ownership: the retained-reservation set and its incursion
observation, the scope-partition logic, the reservation record and its replay
`apply`, and the conflict/holder evaluation are separate clusters that do not
appear in each other's field lists. Tests move with the type each one covers.

**Files:**
- `crates/cargo-berth/src/reservation/mod.rs`
- `crates/cargo-berth/src/reservation/retention.rs`
- `crates/cargo-berth/src/reservation/partition.rs`
- `crates/cargo-berth/src/reservation/record.rs`
- `crates/cargo-berth/src/reservation/conflict.rs`

**Seats:** 3 writers + 0 testers — same shape as phase 9: disjoint type clusters
out of one root, tests moving with their types.
- `impl` — `reservation/mod.rs` and `record.rs`; hub: `reservation/mod.rs`
- `test` — opens as `impl`; `retention.rs` and `partition.rs`
- `review` — opens as `impl`; `conflict.rs`

**Acceptance gate:**
1. `reservation/mod.rs` contains only `mod` declarations, `use`/`pub use`, and
   module documentation.
2. No `too_many_lines` suppression remains under `crates/cargo-berth/src/reservation/`.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 1 added
`Reservation::is_active_for_coordination_run_and_worktree` and phase 7 made it
the single home for that predicate. It is an inherent method on `Reservation`,
so it moves with that type into the record cluster and needs no separate
re-export — re-exporting `Reservation` from the root keeps every caller's path
intact. Phase 7 also placed reservation-id ordering with `ReservationId` in
`ids.rs` precisely so this phase does not have to find a home for it here; do not
move it back. Both anchors moved again after phase 2; confirm them in the
current file before splitting.

---

### Phase 11 — `ledger/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `ledger/mod.rs` declares submodules and re-exports, and carries no logic.

**Spec:** Roughly 2,000 lines past its declarations, beside existing
`constants.rs`, `journal.rs`, `lock.rs`, and `projection.rs`. No
`too_many_lines` suppression lives here, so this phase is purely a move. The
clusters: the `Ledger` handle and its transaction driver; `WorktreeContext` and
its discovery; the identity files (`read_or_create_repo_instance_id`,
`create_or_read_worktree_id`, and the read-only variant); the coordination-run
marker handling; and the path resolution phase 4 moved in —
`normalize_absolute_path`, `canonicalize_through_nearest_existing_ancestor`,
`AbsolutePathNormalizationError` and `AncestorCanonicalizationError` — which
belongs in a new `ledger/path.rs`. The file is 2,559 lines as this phase begins.

`normalize_absolute_path`'s doc comment moves **intact**. It is the only written
record of the rule that separates the two helpers, and phase 4 shipped a defect
from getting that rule backwards: the collapse is textual, so it is sound only
when every component left of a `..` is a real directory. A path whose `..` must
be resolved for real goes to `canonicalize_through_nearest_existing_ancestor`
uncollapsed. Losing that comment in the move would delete the only warning.

**Files:**
- `crates/cargo-berth/src/ledger/mod.rs`
- `crates/cargo-berth/src/ledger/handle.rs`
- `crates/cargo-berth/src/ledger/worktree_context.rs`
- `crates/cargo-berth/src/ledger/identity.rs`
- `crates/cargo-berth/src/ledger/session.rs`
- `crates/cargo-berth/src/ledger/path.rs`

**Seats:** 3 writers + 0 testers — a pure move with four independent clusters and
no suppression to remove, so tests travel with their types.
- `impl` — `ledger/mod.rs`, `handle.rs`, and `session.rs`; hub: `ledger/mod.rs`
- `test` — opens as `impl`; `worktree_context.rs`
- `review` — opens as `impl`; `identity.rs`

**Acceptance gate:**
1. `ledger/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. The existing suite passes unmodified.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** the identity functions were renamed away from
the banned vocabulary before this plan started; keep the current names. Phase 1
reads the harness-session mapping through this module under the mutation lock —
`remove_current_session_mapping` acquires `MutationLock` before removing, and
that ordering is load-bearing, so it moves intact with the handle cluster.
`ledger/journal.rs` no longer carries a `dead_code` suppression — the
`wire_name` method is test-only and exercised — so this module has no suppression
for any phase to remove.

---

### Phase 12 — `board/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `board/mod.rs` declares submodules and re-exports, and carries no
`too_many_lines` or `too_many_arguments` suppression.

**Spec:** `board/mod.rs` is 1,886 lines beside `tests.rs` and `tui.rs`, with
three suppressions: `build` (`:736`, `too_many_lines`), `recorded_answers`
(`:1269`, `too_many_lines`), and `append_authorization_answer` (`:1404`,
`too_many_arguments`, six parameters).

Phase 2 gave this file two more owners than the original split anticipated:
`CompleteBoardReport` and `ReservationLifecycleReport`, plus
`envelope_presentation` and `reservation_lifecycle_presentation`, which render
the complete report as presentation blocks. Each report type and its rendering
belong together in one module; do not leave the report types in the root while
their presentation moves.

Split along row assembly, visibility and omission policy, the
answer/disposition rendering, and the report-and-presentation cluster.
`append_authorization_answer` sits in the answer-rendering cluster: its six
parameters are the audit row's complete input, so give that cluster a semantic
projection type — name it `RecordedAuthorizationConsequenceProjection` —
carrying the recorded authorization and its current consequence
rather than suppressing the count — the type says what the row is, where the
parameter list only says how many pieces it has. `board/tests.rs` is an existing
sibling test module; move each test to sit with the type it covers rather than
leaving a catch-all.

**Files:**
- `crates/cargo-berth/src/board/mod.rs`
- `crates/cargo-berth/src/board/tests.rs`
- `crates/cargo-berth/src/board/rows.rs`
- `crates/cargo-berth/src/board/visibility.rs`
- `crates/cargo-berth/src/board/answers.rs`
- `crates/cargo-berth/src/board/report.rs`

**Seats:** 3 writers + 0 testers — four clusters split cleanly, and
`board/tests.rs` is redistributed rather than owned by a test lane.
- `impl` — `board/mod.rs`, `tests.rs`, `rows.rs`, and `visibility.rs`; hub:
  `board/mod.rs`
- `test` — opens as `impl`; `answers.rs`
- `review` — opens as `impl`; `report.rs`

**Acceptance gate:**
1. `board/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. No `too_many_lines` or `too_many_arguments` suppression remains under
   `crates/cargo-berth/src/board/`.
3. `append_authorization_answer` takes one semantic projection type, not a
   parameter list.
4. The existing suite passes unmodified, including
   `tests/board.rs::populated_board_presentation_carries_the_complete_board_report`.
5. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 4 made `envelope_presentation` route
its empty case through `NonEmptyRenderedBlocks::try_from` to
`EnvelopePresentation::NothingToShow` rather than returning an empty vector, so
that conversion travels with the report cluster when it moves. Phase 1 rendered
the ambiguity outcome in
top-level `output.rs`, not in `board/mod.rs`, so no phase-1 rendering moves here.
Phase 2 added `CompleteBoardReport`, `ReservationLifecycleReport`,
`envelope_presentation`, and `reservation_lifecycle_presentation` to this file;
they are the reason it no longer fits in a shared phase with `gate/`. Phase 7
placed reservation-id ordering with `ReservationId` in `ids.rs`; `board/mod.rs`
calls it and does not re-implement it.

---

### Phase 13 — `gate/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `gate/mod.rs` declares submodules and re-exports and carries no logic.

**Spec:** 1,402 lines beside `install.rs` and `permit.rs`, with no suppression of
its own — a pure move. Split along reference-transaction evaluation, branch
rewrites and re-anchoring, and forced-permit auditing. Tests move with the type
each one covers.

**Files:**
- `crates/cargo-berth/src/gate/mod.rs`
- `crates/cargo-berth/src/gate/reference_transaction.rs`
- `crates/cargo-berth/src/gate/rewrite.rs`
- `crates/cargo-berth/src/gate/audit.rs`

**Seats:** 3 writers + 0 testers — a pure move along three independent
boundaries, tests travelling with their types.
- `impl` — `gate/mod.rs` and `reference_transaction.rs`; hub: `gate/mod.rs`
- `test` — opens as `impl`; `rewrite.rs`
- `review` — opens as `impl`; `audit.rs`

**Acceptance gate:**
1. `gate/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. No suppression is added anywhere under `crates/cargo-berth/src/gate/`.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 7 placed reservation-id ordering with
`ReservationId` in `ids.rs`; `gate/mod.rs` calls it at `:962` and does not
re-implement it. `gate/permit.rs` carries its own `#[allow]` at `:473`, which
belongs to this phase's directory sweep only if this phase moves it; otherwise it
is the final suppression phase's item.

---

### Phase 14 — Remove the remaining suppressions · status: todo

#### Work Order

**Goal:** No suppression remains anywhere in `crates/cargo-berth/`, except the
pre-authorized test-module boilerplate — `clippy::expect_used`, and
`clippy::panic` where the module uses `panic!`.

**Spec:** Four sites survive the earlier phases, in three shapes.

`crates/cargo-berth/tests/board.rs` holds two: a `too_many_lines` at `:881` on
`release_dispositions_remain_resolved_when_trunk_rewrites` (`:885`) and a
`needless_pass_by_value` at `:4385`. The test splits into its arrangement and its
per-disposition assertions; the helper takes its payload by reference.

`crates/cargo-berth/src/cli.rs:584` suppresses `struct_excessive_bools` on the
resolve arguments, whose flags are one mutually exclusive disposition each and
are already grouped by `RESOLVE_DISPOSITION_GROUP`. Replace the flag set with
semantic groups that convert immediately into `ResolveDecision` at the Clap
boundary, so the boolean count disappears rather than being excused. Confine the
raw optionals to one explicitly boundary-owned type —
`UnvalidatedResolveDispositionSelection` — that Clap fills and that converts into
the existing `ResolveDecision` at once, so nothing optional reaches the verb.

`crates/cargo-berth/src/ids.rs:132` carries a
`cfg_attr(not(test), expect(dead_code, …))` on the `uuid_identifier!` macro's
`future` constructor arm — an unused-outside-tests suppression that authors a
reason string, which this plan's binding constraint forbids. Give the constructor
a real consumer, or delete the arm that has none. It is a multi-line attribute:
a single-line `rg 'cfg_attr.*expect'` does not match it.

`crates/cargo-berth/src/ledger/journal.rs` is **no longer a site.** Its
`dead_code` suppression on the macro-generated `wire_name` is already gone — the
method is test-only and exercised — so nothing there remains for this phase.

Then sweep the whole crate and prove the claim: the only `#[allow]`/`#[expect]`
attributes left name `clippy::expect_used` or `clippy::panic` on a
`#[cfg(test)]` module, which
`~/rust/nate_style/rust/test-module-allow-boilerplate.md` pre-authorizes. A
`cfg_attr`-wrapped suppression counts; search for both spellings.

**Files:**
- `crates/cargo-berth/tests/board.rs`
- `crates/cargo-berth/src/cli.rs`
- `crates/cargo-berth/src/ids.rs`

**Seats:** 2 writers + 1 tester — the three sites are in three files, and two of
them are the test suite, so a real test lane exists.
- `impl` — `cli.rs`
- `test` — `tests/board.rs`
- `review` — opens as `impl`; `ids.rs`

**Acceptance gate:**
1. A crate-wide sweep, covering both `#[allow]`/`#[expect]` and `cfg_attr`-wrapped
   forms, shows no `too_many_lines`, `too_many_arguments`, `dead_code`,
   `needless_pass_by_value`, or `struct_excessive_bools` suppression.
2. Every surviving allow names only pre-authorized test lints, and each one's
   module actually uses the lint's pattern — no speculative allows.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.
4. `bash ~/.claude/scripts/delegate/verify.sh final` passes, and
   `~/.claude/scripts/lint/lint mend`, `lint clippy --workspace`, and `lint doc`
   are all clean.

**Constraints from prior phases:** the module phases own every other
non-boilerplate suppression in the crate — phase 8 the two `too_many_lines` sites
in `reconcile.rs`, phase 9 the one in `git/mod.rs`, phase 10 the two in
`reservation/mod.rs`, and phase 12 the three in `board/mod.rs`. If one survives,
it is that phase's defect, not a new item here. Every other `#[allow]` still in
`crates/cargo-berth/src/` names `clippy::expect_used` or `clippy::panic` on a
`#[cfg(test)]` module and is pre-authorized boilerplate. The sites named above
were never owned by an earlier phase and are this phase's own work.

## Gates

- Every phase: `verify.sh test cargo-berth` and `verify.sh lint cargo-berth`.
- Final: `verify.sh final`, plus `lint mend`, `lint clippy --workspace`, `lint doc`.
- No phase adds a suppression. No phase pushes.
