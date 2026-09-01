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

### Phase 5 — `cargo-berth hook post-tool-use` and `hook session-start` · status: done

#### As-built

- `hook post-tool-use` performs drift and, when the response depends on it, assembles the
  live incursion board in the same process, emitting one response object and its exit code.
  `PostToolUseRendering::FeedbackDecidedByLiveIncursionState` (formerly
  `RequiresLiveIncursionBoard`) is the domain decision that verb reads to decide whether to
  call `live_incursion_state()`, not a wire-return marker.
- `hook session-start` reads `BoardModel::envelope_presentation` and branches on
  `RenderedBlocks` / `NothingToShow` / `NotProvided`; the board verb's no-facts constructors
  carry a real presentation, so an unconfigured repository and an unreadable ledger are
  distinguishable without classifying envelope facts.
- An absent or invalid payload session id publishes a no-session selection that cannot fall
  through to an ambient `CARGO_BERTH_SESSION_ID`. Both verbs bind through one helper, pinned
  by `post_tool_use_runs_drift_under_the_payload_session_not_the_ambient_one`;
  session-start's own output cannot observe it, since `journal_mutation_actor_for` discards
  the `EditAuthorization` and keeps only the worktree.
- `hook/mod.rs` is a shared protocol across three events: payload types,
  `HookWorkingDirectorySelection` (into which `PostToolUseWorkingDirectory` collapsed),
  harness session identity, rendering helpers.
- The pre-tool-use edit target reaches `canonicalize_through_nearest_existing_ancestor`
  uncollapsed, and `WorktreeRelativeEditName` rebuilds the worktree-relative name from
  `Component::Normal` alone, so a surviving `..` answers `OutsideCoordinationDomain`.
- Fail-open causes stay distinct: invalid payload and unavailable working directory emit
  different messages under `continue: true`; a rejected request exits 5, a contended ledger 6.
- `tests/front_end_corpus.rs` covers the corpus lane by asserted partition —
  `HOOK_ACCEPTANCE_TEXT_COMPARED_ENTRIES` and `HOOK_CORPUS_ENTRIES_WITHOUT_A_TEST` disjoint,
  together exhausting the fixture's `PostToolUse`/`SessionStart` entries — not by count.

**Files:**
- `crates/cargo-berth/src/hook/mod.rs` — shared hook event protocol.
- `crates/cargo-berth/src/hook/post_tool_use.rs` — the verb, its two fail-open texts, `INVALID_PAYLOAD_DETAIL`.
- `crates/cargo-berth/src/hook/session_start.rs` — the verb and its presentation branch.
- `crates/cargo-berth/src/hook/pre_tool_use.rs` — the edit decision, filesystem-resolved parents.
- `crates/cargo-berth/src/cli.rs`, `crates/cargo-berth/src/output.rs` — dispatch and rendering; both still carry `--post-tool-use-payload`.
- `crates/cargo-berth/tests/hooks.rs` — acceptance suite for all three hook verbs.
- `crates/cargo-berth/tests/front_end_corpus.rs` — corpus partition and citation guard.
- `crates/cargo-berth/tests/engine_instructions.rs` — real-binary scenarios for both verbs.

**Binds later work:** the installed `berth_post_bash.sh` still drives the two-step
live-incursion round trip, so the `--post-tool-use-payload` plumbing in `cli.rs` and
`output.rs` is live on purpose until the cutover that installs the hook wrappers deletes
both halves together. The engine's invalid-payload sentence and that shell hook's
`INVALID_PAYLOAD_DETAIL` are the same text in two places and must change in one step or
front-end parity breaks. `hook/mod.rs` is shared across three events, so a new hook event
extends it; `tests/front_end_corpus.rs` is a partition assertion, so whatever retires the
fixture retires that structure with it.

**Gotchas:**
- A coverage table that counts can go green while lying:
  `every_cited_acceptance_test_exists_in_the_suite` `include_str!`s `tests/hooks.rs` and
  fails on a cited name that file does not define — a test moved elsewhere loses the guard.
- The `normalize_absolute_path` collapse is textual, sound only when every component left of
  a `..` is a real directory: it applies to a working directory the harness reports itself
  sitting in, never to a path a payload names as an edit target.
- Corpus entry `without_message#4` is unreachable — `reconcile.rs:669` sweeps coordination
  run markers with a predicate byte-for-byte identical to `validate_marker`'s acceptance
  predicate at `coordination_identity.rs:670`, deleting exactly what validation would reject.
- The invalid-payload sentence names valid JSON, `tool_name` Bash and a 1-to-256 character
  `session_id`, but not `cwd` — and a non-string `cwd` is what produces it.

**Ruled out:**
- Satisfying the corpus gate by lowering a count — fourteen of fifteen uncovered entries are
  unproducible by any real binary.
- Deleting `PostToolUseRendering::FeedbackDecidedByLiveIncursionState` at the cutover — it is
  the internal round trip now, not the shell handoff.
- Routing the invalid-payload sentence to next-items — the shell half disappears with the
  cutover, leaving one sentence and one edit.
- Adding a machine-readable action list to the session-start response — no consumer.

### Phase 6 — Retire the coordinator and the generated validators · status: done

#### As-built

- The installed front end is three hook wrappers: each checks that `cargo-berth` is on `PATH` and then `exec`s `cargo-berth hook <event>`, so the engine writes every byte of the harness protocol response — byte-identical on stdout, stderr and exit status to invoking the engine directly for all three events. The one policy each wrapper still states alone is its binary-absent failure mode: pre-edit refuses (exit 2, stderr notice), post-bash and session-start state it and exit 0, since neither can refuse what it reports on. Both notices are static JSON written with `printf`, so they hold when nothing else on the path does.
- `Command::execute` returns which hook answers rather than running it — `CommandOutputOwnership::HookOwnsItsResponse(HookCommand)`, with `Cli::run` calling `HookCommand::write_response()` one frame later. All 16 entries of `cli.rs`'s `ALL: [Self; 16]` route table assert their real ownership from a unit test that reads no stdin.
- `CommandResultReporting` has three answers: `Envelope`, `HookProtocol(HookCommand)`, and `GitHookProtocol` for `__reference-transaction` and `__refresh-managed-hook-after-trunk-deletion`, which return from `Cli::run` before any envelope exists.
- The hidden `--post-tool-use-payload` two-step route is refused by the command line rather than merely unused; `PostToolUseRendering::FeedbackDecidedByLiveIncursionState` survives as the domain decision `hook/post_tool_use.rs:237` reads to choose whether to consult `live_incursion_state()`. `INVALID_PAYLOAD_DETAIL` now names `cwd` alongside JSON validity, `tool_name` and `session_id`.
- `claim_state.py`, the generated Python status tables and the generated jq validator are deleted; `consumer_artifacts` is gone from `output_contract.rs` and from the generated contract, swept across four regions of `json-contract.md`. `/sync`, `/plan:delegate` and `/plan:delegate_checkpoint` invoke the binary directly, resolving the repository root from a subdirectory, propagating the harness session id, invoking once, and printing the engine's rendered text verbatim.
- The Python suite is three modules — the fixture, the surviving wrapper tests, and the timing tests re-keyed to binary and wrapper availability. Its cells now build durable ledger states rather than mutating the journal between two front-end calls, which names two engine answers the retired two-call route had obscured: `could not read the reservation ledger` and `REPLAY HARD STOP: duplicate_incursion_incident`.

**Files:**
- `crates/cargo-berth/src/cli.rs` — the command line, its three route tests, and the `ALL: [Self; 16]` route table
- `crates/cargo-berth/src/output_contract.rs` — the wire contract without consumer artifacts
- `crates/cargo-berth/tests/front_end_corpus.rs` — the frozen front-end oracle, its three coverage tables, and the `MINIMUM_FROZEN_CORPUS_ENTRIES = 50` ratchet
- `docs/cargo-berth/json-contract.md` — the wire contract independent consumers read
- `~/.claude/scripts/berth/install/hooks/*.sh` — the three wrappers (outside this repository)
- `~/.claude/scripts/berth/install/install.sh` — build, publish, roll back; no generated-artifact staging, validation, or second rollback arm
- `~/.claude/scripts/berth/tests/{installed_front_end,test_hook_rendering,test_hook_timing}.py` — the front-end suite (outside this repository)

**Binds later work:** `CommandOutputOwnership::HookRendered(ExitCode)` no longer exists anywhere. `cli.rs` changed substantially — the two-step route removed, the ownership enum restructured, three unit tests added. Nothing under `tests/` asserts over the generated output contract any more; the `GENERATED_CONTRACT_JSON` include is gone from `front_end_corpus.rs`. Because the wrappers are pass-throughs, changing a hook's rendered text changes what users see with no front-end edit and no front-end file to forget.

**Gotchas:**
- `MINIMUM_FROZEN_CORPUS_ENTRIES = 50` is a ratchet: deleting a corpus entry fails `the_frozen_corpus_never_shrinks` by name, deliberately, because the coverage partition alone stays balanced when an entry and its row go together.
- `POST_TOOL_USE_BOUND_SECONDS = 0.20` was not remeasured and still describes the retired two-call front end. The cold-page gate demands zero resident pages for `git`, and any other process on the machine executing git faults them back in. Neither widen the bound nor loosen the gate to make a run green — both turn a refusal to measure into a false measurement.
- Break-and-restore is what busts the clippy cache correctly: a run that fails to compile leaves no cached success. `cargo clean -p` costs a full rebuild and wipes a `target/` other processes may be using.
- `__refresh-managed-hook-after-trunk-deletion` has no command-line test in this crate, and the `Cli::run` doc comment says so rather than claiming coverage it lacks.

**Ruled out:**
- Asserting the exit-code half through `Cli::run` — `ExitCode` has no `PartialEq`, so the comparison would run over `Debug` strings in an undocumented format and could go tautological; `tests/drift.rs` and `tests/overlap.rs` already pair process exit against the envelope field at four distinct non-zero codes.
- Serializing the Rust suite against the working-directory hazard — nextest runs each test in its own process, `verify.sh` and CI use it exclusively, and the drop guard restores on panic.
- Amending `berth-fix.md`'s `claim_state` references — that is a prior plan's as-built record, and correcting it belongs to that plan's closeout.
- Giving both git-invoked commands a `HookRendered(ExitCode)` return to collapse the reporting enum to two states — the `HookOwnsItsResponse` restructure reaches the same end.

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

Reservation-id ordering by rendered string appears six times:
`verb/claim.rs:453` (`sort_by_cached_key`), `drift/ordering.rs:12`,
`output.rs:3887`, `board/mod.rs:945`, `gate/mod.rs:962`, and `reconcile.rs:1708`,
which sorts by `predecessor_id.to_string()` where `GraphPredecessor::reservation_id`
is a `ReservationId` (`edge/mod.rs:220`). `drift::ordering` is `pub(super)` to
`drift`, so no other caller can reach it. Give the ordering one home with
`ReservationId` in `crates/cargo-berth/src/ids.rs`, and encode the guarantee in
the type rather than in a comment: a `Vec<ReservationId>` that four call sites
promise to have sorted is not a guarantee, and phase 1's candidate list documents
its determinism only in prose. Introduce a named ordered collection —
`WireOrderedReservationIds` or an equally explicit name — that can only be
constructed sorted, and have the wire-facing producers hold it.

**The ordering has two guarantees, not one, and the type must express both.**
`drift/ordering.rs` exposes `sort_reservation_ids` (`:12`) and
`sort_and_deduplicate_reservation_ids` (`:17`). Four callers require the
deduplicating form — `drift/report.rs:110`, `drift/classification.rs:96`, `:123`,
and `:161` — and four require only the sort: `verb/claim.rs:453`,
`output.rs:3887`, `board/mod.rs:945`, and `gate/mod.rs:962`. A single sorted-only
collection leaves `drift` holding a wrapper, which is the duplicate this phase
exists to remove. Decide the shape here rather than mid-phase: the collection
carries both constructions, and each one names the guarantee it makes.

This is behavior-preserving. Every existing test passes unmodified, and the
ordering the wire already emits does not change.

**Files:**
- `crates/cargo-berth/src/ids.rs`
- `crates/cargo-berth/src/reservation/mod.rs`
- `crates/cargo-berth/src/verb/claim.rs`
- `crates/cargo-berth/src/drift/ordering.rs`
- `crates/cargo-berth/src/drift/selection.rs`
- `crates/cargo-berth/src/drift/report.rs`
- `crates/cargo-berth/src/drift/classification.rs`
- `crates/cargo-berth/src/output.rs`
- `crates/cargo-berth/src/board/mod.rs`
- `crates/cargo-berth/src/gate/mod.rs`
- `crates/cargo-berth/src/reconcile.rs`

**Seats:** 3 writers + 0 testers — no caller can convert until
`WireOrderedReservationIds` exists, so every seat waits on `impl`'s first commit
of the type before its own edits compile. Plan for that: `impl` lands the type
first and says so on the board, and the other two read the tree before starting.
- `impl` — `ids.rs`, `verb/claim.rs`, `output.rs`, `board/mod.rs`,
  `gate/mod.rs`; hub: `ids.rs` (the type lands first; its callers convert behind it)
- `test` — the drift lane: `drift/ordering.rs`, `drift/selection.rs`,
  `drift/report.rs`, `drift/classification.rs`
- `review` — the `reservation/mod.rs` eligibility consolidation, and
  `reconcile.rs:1708`

**Acceptance gate:**
1. A crate-wide sweep finds one implementation of the run-and-worktree
   eligibility predicate and one of the reservation-id ordering, with no inline
   restatement of either. The sweep will also reach `board/mod.rs:1181`, which
   sorts `worktree_heads` by `worktree_id.to_string()` — the same rendered-string
   ordering idiom over `WorktreeId` rather than `ReservationId`. **That site is
   out of scope**: this phase gives `ReservationId` one ordering home, and
   widening to a second id type is a different consolidation. Leave it, and do
   not let the sweep stall on it. `reconcile.rs:1708` is **in** scope by the same
   test — it orders `ReservationId` — and is routed through the new collection
   like every other site.
2. The ordered collection cannot be constructed unsorted, and that is proven by
   a test rather than asserted in a comment.
3. The existing suite passes unmodified — this phase changes no behavior.
4. `generated_artifacts_are_reproducible_from_the_checked_in_contract` passes
   without the regenerate environment variable, proving the wire did not move.
5. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 2 **did** add a third call to the
eligibility method — the `check --reservation` selector's explicit reservation
selection — so this phase consolidates three call sites, not two, and that
selector uses the single home this phase establishes. The three are
`crates/cargo-berth/src/verb/claim.rs:394`, `:422`, and `:442`; `:365` is the
`FirstTouchReservationSelection` enum declaration, not a call. This consolidation runs before the module phases deliberately:
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
projection type — name it `RecordedAuthorizationConsequence` —
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
re-implement it. `gate/permit.rs` carries an `#[allow]` at `:473`, but it is
`clippy::expect_used` on a `mod tests` with the reason "tests should panic on
unexpected values" — pre-authorized test boilerplate, which is why the final
suppression phase counts four surviving sites and does not list it. It is
nobody's item; leave it.

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

`crates/cargo-berth/src/cli.rs:585` suppresses `struct_excessive_bools` on the
resolve arguments, whose flags are one mutually exclusive disposition each and
are already grouped by `RESOLVE_DISPOSITION_GROUP`. Replace the flag set with
semantic groups that convert immediately into `ResolveDecision` at the Clap
boundary, so the boolean count disappears rather than being excused. Confine the
raw optionals to one explicitly boundary-owned type —
`UnvalidatedResolveDispositionSelection` — that Clap fills and that converts into
the existing `ResolveDecision` at once, so nothing optional reaches the verb.

`ResolveArguments.why` (`crates/cargo-berth/src/cli.rs:645`) is a bare
`Option<String>` carrying a domain fact — the justification for a deliberate
abandonment or an orphan retirement — so name what it converts into rather than
letting a `String` reach `ResolveDecision`. The converted form belongs beside the
disposition it justifies, since a justification with no disposition is not a
state this command line can reach.

**The resolve route is now a wire fact, not only a parser fact.** Phase 6 added
`CommandLineRoute::Resolve.arguments()` (`crates/cargo-berth/src/cli.rs:2070`),
which builds the literal argv `resolve <id> --recovered --json` for the recovery
command the engine prints. Replacing the resolve flag set changes that argv, so
the route table is part of this phase's surface and its acceptance gate.

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
3. `CommandLineRoute::Resolve.arguments()` still builds a runnable resolve
   command line, and the three `cli.rs` route tests phase 6 added still pass
   unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.
5. `bash ~/.claude/scripts/delegate/verify.sh final` passes, and
   `~/.claude/scripts/lint/lint mend`, `lint clippy --workspace`, and `lint doc`
   are all clean.

**Constraints from prior phases:** phase 6 added three unit tests to
`crates/cargo-berth/src/cli.rs` that this phase must keep compiling and true:
`only_the_hook_routes_answer_a_protocol_instead_of_an_envelope`,
`every_command_line_route_answers_through_the_output_ownership_it_declares`
(`:2593`), and the `ALL: [Self; 16]` route table they iterate. The resolve route
must still report `CommandResultReporting::Envelope(CommandVerb::Resolve)` after
the flag set is replaced. The module phases own every other
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
