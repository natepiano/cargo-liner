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
  `~/.claude/scripts/berth/` — `install/install.sh` (builds and publishes the
  binary, restoring the preceding one if publication fails), the three wrappers
  `install/hooks/berth_pre_edit.sh`, `install/hooks/berth_post_bash.sh`, and
  `install/hooks/berth_session_start.sh`, plus `work_order.py` and the Python
  suite under `tests/`. Each wrapper checks that `cargo-berth` is on `PATH` and
  then `exec`s `cargo-berth hook <event>`, so the engine writes every byte a user
  reads and the two cannot disagree about an outcome. The coordinator
  `claim_state.py`, the generated Python status tables, and the generated jq
  validator no longer exist. A phase that changes this layer says so in its
  summary: the checkpoint commit cannot carry files outside the repository.
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

  **Point `CARGO_TARGET_DIR` at a private scratch directory before every gate
  run.** Most phases open three concurrent writers against one `target/`, and a
  shared one has produced four distinct false signals: a clippy exiting 0 with no
  `Checking` line (a cache hit, and `cargo clean -p` does not reliably bust it —
  a peer repopulates it between the clean and the run); a `check` that is blind to
  `cfg(test)` code because it builds `--bins`; a test binary deleted by a peer
  mid-run, reading as every test failing in seconds; and a wall-clock timing test
  that fails under contention and passes alone. A private target directory
  insulates against all four, and it is the only way a green result is provably a
  compile of the seat's own bytes. Report the captured exit code, the test counts,
  and whether a real `Compiling` / `Checking cargo-berth` line appeared — an exit
  0 with no such line is not a pass.

  **Delete the scratch directory as soon as its result is recorded.** A cold build
  of this crate is large, several seats run at once, and every phase repeats it;
  nothing else reclaims them. Leaving one behind per seat per round fills the disk
  within a few phases, and a full disk stops every command in the session, not just
  the build. Removing it costs one cold rebuild next time and nothing else.
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

### Phase 7 — One home for run eligibility and reservation-id ordering · status: done

#### As-built

Run eligibility is two inherent methods on `Reservation` in `reservation/mod.rs`. `is_active_for_coordination_run(CoordinationRunId)` holds the `Active` lifecycle test and the run term; `is_active_for_coordination_run_and_worktree(CoordinationRunId, WorktreeId)` delegates to it and adds the worktree term, so the narrower predicate is structurally a subset and the lifecycle test is written once. Every site in the crate that means "active for this run (and worktree)" calls one of the two.

Reservation-id ordering is one type, `WireOrderedReservationIds` in `ids.rs`. Its `sorted` and `sorted_and_deduplicated` constructors key on `ReservationId::wire_ordering_key()`, its hand-written `Deserialize` sorts on the way in, and its `Serialize` forwards to the inner `Vec`; readers go through `as_slice`, `const fn is_empty`, and `into_vec`. The type cannot be constructed unsorted, and a test in `ids.rs` proves it. **Seven** call sites route through it, not six: the seventh is a composite sort in `reconcile.rs` keyed on `predecessor_reservation_id.wire_ordering_key()`. `ResolvedDriftSubjects.reporting`, `DriftSelectionError::AmbiguousActiveReservations`, `CompleteBoardReport::reservation_ids()`, and `FirstTouchReservationSelectionPayload.candidate_reservation_ids` hold the type rather than a bare `Vec`.

Two printed surfaces order ascending rather than by ledger order: the ambiguity message naming candidate reservations, and the reporting list in `drift --json`. Nothing asserts either order — `tests/drift.rs:2020` checks only that both identifiers appear.

**Files:**
- `crates/cargo-berth/src/ids.rs` — `WireOrderedReservationIds`, its two constructors, accessors, hand-written `Serialize`/`Deserialize`, and `ReservationId::wire_ordering_key`.
- `crates/cargo-berth/src/reservation/mod.rs` — the two eligibility predicates.
- `crates/cargo-berth/src/drift/selection.rs` — both drift carriers hold the ordered type; `drift/execution.rs`, `drift/classification.rs`, `drift/ordering.rs`, `drift/report.rs` read through `as_slice()` / `is_empty()`.
- `crates/cargo-berth/src/reconcile.rs` — an ordered build inside `successor_incorporation_evidence`, and the composite sort keyed on `wire_ordering_key`.
- `crates/cargo-berth/src/output.rs`, `board/mod.rs`, `gate/mod.rs`, `verb/claim.rs` — ordered producers and consumers.
- `crates/cargo-berth/src/verb/release.rs`, `coordination_identity.rs` — eligibility call sites.

**Binds later work:** The ordering idiom has exactly one implementation, `WireOrderedReservationIds::sorted`; any later phase that moves a sort routes it there rather than adding a second. `WireOrderedReservationIds` reaches the generated output contract through `FirstTouchReservationSelectionPayload`, so an edit to that field, its attribute, or the type's `Serialize` moves the wire (see Gotchas). Both eligibility methods must travel together when `Reservation` moves out of `reservation/mod.rs`, and their intra-doc links must still resolve — moved intra-doc links fail as rustdoc errors that the test and lint gates do not surface, so that move needs a doc lint. Both `reconcile.rs` edits sit inside `successor_incorporation_evidence`, and the split of that function carries them intact rather than re-deriving them. `CompleteBoardReport::reservation_ids()` returns the ordered type, and that signature moves with the report cluster when `board/mod.rs` splits. `gate/mod.rs:953`, inside `blocking_reservations`, calls `sorted_and_deduplicated`; dropping the deduplication changes what the gate reports. Line anchors in `ids.rs`, `reservation/mod.rs`, `reconcile.rs`, `output.rs`, `board/mod.rs`, and `gate/mod.rs` all moved.

**Gotchas:**
- `WireOrderedReservationIds` is `pub(crate)` but is **not** crate-internal: it is the type of `FirstTouchReservationSelectionPayload.candidate_reservation_ids` in `output.rs`, which appears in `docs/cargo-berth/generated/output-contract.json`. The serialized bytes are unchanged **only** because that field carries `#[schemars(with = "Vec<String>")]` and the type serializes transparently to its inner `Vec`. Both halves are required — dropping either moves the wire. The same pairing is why the type needs no `JsonSchema` derive of its own.
- `OutputEnvelope.reservations` and `OutputEnvelope.blocked_by` keep `Vec<ReservationId>`: they also receive the unsorted `DriftReport::reservation_ids()`, so converting them would change the wire.
- A crate-wide sweep for a boolean predicate must cover its negated spelling. The eligibility sweep keyed on the `==` comparison, and one site writing the same condition in De Morgan form with `!=` survived it.
- `drift/selection.rs:147-149` is worktree-only with no run term, and `board/mod.rs:1181` sorts `WorktreeId` rather than `ReservationId`. Neither restates what lives here; both are correct as they stand.

**Ruled out:**
- Converting `OutputEnvelope.reservations` / `blocked_by` to the ordered type — they also carry unsorted input, so it would move the wire.
- Widening the two-field predicate to cover `drift/selection.rs:147-149` — that site asks a worktree question with no run term.
- Retyping `board/mod.rs:1181`'s `WorktreeId` sort — a different id type, scoped out on purpose.
- Leaving `drift` a sorted-only wrapper — the collection carries both the sorting and the deduplicating construction, each naming its guarantee.

### Phase 8 — Split the reconciliation planners · status: done

#### As-built

`reconcile.rs` carries no `too_many_lines` suppression, and behavior is
unchanged — the existing suite passes unmodified.

`build_plan` is two named stages. `observe_repository_facts` reads the repository
once — worktree registry, trunk, batched integration reachability, evidence
observations — and returns `ObservedRepositoryFacts`, which also carries
`trunk_resolution_calls` so the count travels with the resolution it reports.
`reconcile_observed_reservations` applies those facts to every retained
reservation and returns `ReconciledReservations` (changes, alert subjects,
snapshots). `LedgerCheckoutIdentity` names the checkout whose recorded worktree a
reconciliation classifies against.

`successor_incorporation_evidence` is 22 lines over three named steps:
`predecessor_successor_evidence_subjects` walks the predecessor subjects,
`classify_successor_incorporation` groups descendant commits and resolves each
predecessor through `predecessor_successor_incorporation`, and
`settle_pending_scoped_patch_comparisons` performs the budgeted comparisons.
`PredecessorEvidenceStanding::{Measurable, NoProtectedTip}` replaces a `continue`
buried in a match arm; `PredecessorSuccessorReachability` pairs the protected-tip
and phase-start classifications and owns `phase_start_target_histories()`;
`PendingScopedPatchCandidateContext` holds the per-predecessor facts every
candidate copies and builds them through `.candidate(head)`.

`ReconciliationSuccessorScopedPatchEvaluationBudget` (the enum at `:490`, with
`evaluate` at `:499`) reaches exactly one extracted function,
`settle_pending_scoped_patch_comparisons`; the first two steps never see it, so
single admission is unchanged. It is distinct from
`ReconciliationScopedPatchEvaluationBudget` (the struct at `:480`) — both exist.

**Files:**
- `crates/cargo-berth/src/reconcile.rs` — the two reconciliation planners as
  named stages, with the carrier types that pass values between them.

**Binds later work:** `trunk_resolution_calls` is a wire field, not an internal
counter — it reaches users as `board --json` →
`payload.data.git_cost.trunk_resolution_calls`, appears twice in the generated
output contract, and is frozen in the front-end corpus fixture. Its board-side
renderers move in the phase that turns `board/mod.rs` into a table of contents,
and the git-cost assertions in `tests/board.rs` are what prove the field survived
that move.

**Gotchas:**
- `SuccessorIncorporationClassification`'s two fields are positionally coupled:
  every pending candidate's `predecessor_index` is a position in
  `by_predecessor`, resolved by indexing when its comparison settles. Filtering,
  reordering, or deduplicating `by_predecessor` between construction and
  settlement misaddresses every later candidate — rebuild the indices with it or
  do neither. Nothing in the type system holds this.
- `resolved_successor_heads` returns heads in the order the successors were
  supplied, not in ordering-graph order.
- `phase_start_target_histories` is a bare map whose *missing* key means "no
  proven first-parent interval — fall back to git queries", not "not computed".
  The same empty map arises from `AncestorObjectUnknown` and from a classified
  head that is `NotDescendant`.
- `drift_observation_events_after_current_marker_sweep` is at `reconcile.rs:813`.
  The earlier phase that recorded the coordination-marker sweep cites its former
  location, `:669`, which this split moved; `:669` is now unrelated code.

**Ruled out:**
- Copying the scoped-patch budget into each extracted function — it would break
  the single-admission guarantee, so it stays threaded through one owner.
- Recomputing `phase_start_target_histories` before the protected-tip match, as
  the pre-split code did; computing it inside the `Classified` branch is
  behavior-identical and skips it for a case that discarded the result anyway.

### Phase 9 — `git/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `git/mod.rs` declares submodules and re-exports, and carries no logic
and no `too_many_lines` suppression.

**Spec:** The root holds roughly 3,610 lines past its declarations (file 3,716,
imports end `:104`) — the largest offender in the crate — beside existing `command.rs`, `constants.rs`, and
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
- `impl` — `git/mod.rs` and `reachability.rs`; hub: `git/mod.rs` (the root, which
  `impl` alone writes)
- `test` — opens as `impl`; `patch.rs` and `conflict.rs`
- `review` — opens as `impl`; `refs.rs` and `discovery.rs`

**No seat writes another seat's file.** The pre-edit hook claims paths per
session, so an edit into a peer's claimed file is blocked rather than merged —
which makes a partition where three seats each delete their own cluster from one
root unexecutable. `impl` therefore owns the **entire** root transformation:
every `mod` declaration, every re-export, and every deletion. The other seats
create only their own submodule files, reading the code they move from the root
at `HEAD` (`git show HEAD:<root path>`) rather than from the file `impl` is
rewriting. Nothing serializes and no seat waits for a skeleton, because no seat
outside `impl` ever touches the root. Phase 7 paid for the older arrangement:
two seats converting callers of the same item at once produced a red where each
seat's own files were clean and neither error was its own.

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

**Spec:** Roughly 3,055 lines past its declarations (file 3,125, imports end
`:72`), beside existing `constants.rs`, `evidence.rs`, and `lifecycle.rs`. Two
suppressions live here: `apply` (`:1018`, 150 lines) and a `Display::fmt` on
`ReservationReplayError` (`impl` at `:2113`, 103 lines). The `fmt` is an exhaustive match over a large enum — split
it by giving each variant family its own renderer, keeping the outer match as the
dispatch.

Split by type ownership: the retained-reservation set and its incursion
observation, the scope-partition logic, the reservation record, the replay
machinery, and the conflict/holder evaluation are separate clusters that do not
appear in each other's field lists. Tests move with the type each one covers.

**`apply` stays with its type; only the error moves.** `apply` (`:1018`) is an
inherent method on `RetainedReservationSet`, whose two `impl` blocks are `:656`
(closing before `impl AuthorizedEditingIdentity` at `:1711`) and `:2007`, and
whose fields are private (`:345`-`:346`). Its sixteen `apply_*` helpers span
`:1169` (`apply_incursion_journal_event`) to `:1603` (`apply_replacement`), so
they bracket the range a narrower reading would move. Moving that machinery into
a separate module would split one type's inherent impl across three files and
force its private fields to `pub(super)` — a widening bought for nothing, since
`apply`'s 150 lines must be split into named steps either way to clear gate 2.
So `apply` and every `apply_*` helper travel with `RetainedReservationSet` into
the retention cluster, both `impl` blocks together.

`reservation/replay.rs` still exists, and holds `ReservationReplayError` and its
103-line `Display` (`:2113`) — which are in no other cluster and are where the
`fmt` split lands. The hub owner takes `retention.rs` and `replay.rs` together,
so gates 2 and 3 still turn on one seat rather than two.

**Files:**
- `crates/cargo-berth/src/reservation/mod.rs`
- `crates/cargo-berth/src/reservation/retention.rs`
- `crates/cargo-berth/src/reservation/partition.rs`
- `crates/cargo-berth/src/reservation/record.rs`
- `crates/cargo-berth/src/reservation/replay.rs`
- `crates/cargo-berth/src/reservation/conflict.rs`

**Seats:** 3 writers + 0 testers — same shape as phase 9: disjoint type clusters
out of one root, tests moving with their types.
- `impl` — `reservation/mod.rs`, `retention.rs`, and `replay.rs`; hub:
  `reservation/mod.rs` (the root, which `impl` alone writes)
- `test` — opens as `impl`; `partition.rs` and `record.rs`
- `review` — opens as `impl`; `conflict.rs`

Retention and replay go to the hub owner together because between them they hold
both suppressions, so gates 2 and 3 turn on one seat rather than two.

**No seat writes another seat's file.** The pre-edit hook claims paths per
session, so an edit into a peer's claimed file is blocked rather than merged —
which makes a partition where three seats each delete their own cluster from one
root unexecutable. `impl` therefore owns the **entire** root transformation:
every `mod` declaration, every re-export, and every deletion. The other seats
create only their own submodule files, reading the code they move from the root
at `HEAD` (`git show HEAD:<root path>`) rather than from the file `impl` is
rewriting. Nothing serializes and no seat waits for a skeleton, because no seat
outside `impl` ever touches the root. Phase 7 paid for the older arrangement:
two seats converting callers of the same item at once produced a red where each
seat's own files were clean and neither error was its own.

**Acceptance gate:**
1. `reservation/mod.rs` contains only `mod` declarations, `use`/`pub use`, and
   module documentation.
2. No `too_many_lines` suppression remains under `crates/cargo-berth/src/reservation/`.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.
5. `~/.claude/scripts/lint/lint doc` passes. This split moves items that carry
   intra-doc links across module boundaries, and a link that stops resolving is a
   rustdoc error rather than a compile error — so it is invisible to gates 3 and
   4. Catch it here rather than leaving it for the final phase.

**Constraints from prior phases:** phase 7 made run eligibility **two** methods
on `Reservation`, and they must travel together into the same module:

- `is_active_for_coordination_run` (`:1867`) holds the `Active` lifecycle test
  and constrains the run without the worktree, deliberately, so a run holding
  live work in a second worktree still answers `true`.
- `is_active_for_coordination_run_and_worktree` (`:1880`) delegates to it and
  adds the worktree term.

Splitting them across modules re-creates the duplicate lifecycle test phase 7
removed. Both are inherent methods on `Reservation`, so they move with that type
into the record cluster and need no separate re-export — re-exporting
`Reservation` from the root keeps every caller's path intact. Their callers reach
outside `reservation/` (`coordination_identity.rs`, `verb/release.rs`,
`drift/selection.rs`, `verb/claim.rs`), so the re-export is what keeps those
paths working. Their intra-doc links to each other, and
`RetainedReservationSet::has_other_active_reservation`'s link to the run-only
form (`:1003`), must all still resolve after the move — that is what gate 5
checks.

Phase 7 also placed reservation-id ordering with `ReservationId` in `ids.rs`
precisely so this phase does not have to find a home for it here; do not move it
back. Every anchor above was confirmed against the tree after phase 8; re-confirm
before relying on one, since phases 9 and 10 both edit files this plan cites.

---

### Phase 11 — `ledger/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `ledger/mod.rs` declares submodules and re-exports, and carries no logic.

**Spec:** Roughly 2,458 lines past its declarations (file 2,559, imports end
`:101`), beside existing `constants.rs`, `journal.rs`, `lock.rs`, and
`projection.rs`. That makes it the second-largest root in the crate: carrying no
`too_many_lines` suppression makes this a pure move, not a light one. The
clusters: the `Ledger` handle and its transaction driver; `WorktreeContext` and
its discovery; the identity files (`read_or_create_repo_instance_id`,
`create_or_read_worktree_id`, and the read-only variant); the coordination-run
marker types — `DetachedCoordinationRunMarker` (`:202`),
`CoordinationRunMarkerAtRetirement` (`:209`),
`DetachedCoordinationRunMarkerDisposition` (`:217`),
`EnvironmentCoordinationRunSelection` (`:282`), `CoordinationRunMarkerRemoval`
(`:312`), and `impl DetachedCoordinationRunMarker` (`:650`) — which go to
`ledger/coordination_run_marker.rs`; and the path resolution phase 4 moved in —
`normalize_absolute_path`, `canonicalize_through_nearest_existing_ancestor`,
`AbsolutePathNormalizationError` and `AncestorCanonicalizationError` — which
belongs in a new `ledger/path.rs`.

The marker module is named for what it owns, not for how it is reached. `session`
would be the wrong name twice over: it describes the caller rather than the
contents, and `crate::session` already means harness-session identity, a
different concept this module does not touch.

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
- `crates/cargo-berth/src/ledger/coordination_run_marker.rs`
- `crates/cargo-berth/src/ledger/path.rs`

**Seats:** 3 writers + 0 testers — a pure move with five independent clusters and
no suppression to remove, so tests travel with their types.
- `impl` — `ledger/mod.rs` and `handle.rs`; hub: `ledger/mod.rs` (the root, which
  `impl` alone writes)
- `test` — opens as `impl`; `coordination_run_marker.rs` and `worktree_context.rs`
- `review` — opens as `impl`; `identity.rs` and `path.rs`

**No seat writes another seat's file.** The pre-edit hook claims paths per
session, so an edit into a peer's claimed file is blocked rather than merged —
which makes a partition where three seats each delete their own cluster from one
root unexecutable. `impl` therefore owns the **entire** root transformation:
every `mod` declaration, every re-export, and every deletion. The other seats
create only their own submodule files, reading the code they move from the root
at `HEAD` (`git show HEAD:<root path>`) rather than from the file `impl` is
rewriting. Nothing serializes and no seat waits for a skeleton, because no seat
outside `impl` ever touches the root. Phase 7 paid for the older arrangement:
two seats converting callers of the same item at once produced a red where each
seat's own files were clean and neither error was its own.

**Acceptance gate:**
1. `ledger/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. The existing suite passes unmodified.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.
4. `~/.claude/scripts/lint/lint doc` passes. This root is the only one of the
   five module phases that carries intra-doc links across a boundary this split
   moves, and a link that stops resolving is a rustdoc error rather than a
   compile error — invisible to gates 2 and 3.

**Constraints from prior phases:** the identity functions were renamed away from
the banned vocabulary before this plan started; keep the current names. Phase 1
reads the harness-session mapping through this module under the mutation lock —
`remove_current_session_mapping` acquires `MutationLock` before removing, and
that ordering is load-bearing, so it moves intact with the handle cluster.
`ledger/journal.rs` no longer carries a `dead_code` suppression — the
`wire_name` method is test-only and exercised — so this module has no suppression
for any phase to remove.

Two intra-doc links cross this split and are what gate 4 exists for:
`:860`, inside `impl Ledger`, links `[Enrollment]` and `[BerthConfig::read]` and
travels to `handle.rs`; `:1762` carries `normalize_absolute_path`'s load-bearing
rule and links `[canonicalize_through_nearest_existing_ancestor]`, which keeps
resolving only because both helpers land in `path.rs` together. Splitting those
two helpers across modules breaks the link and deletes the warning.

---

### Phase 12 — `board/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `board/mod.rs` declares submodules and re-exports, and carries no
`too_many_lines` or `too_many_arguments` suppression.

**Spec:** `board/mod.rs` is 1,886 lines beside `tests.rs` and `tui.rs`, with
three suppressions: `build` (`fn` at `:737`, its `#[allow]` at `:733`,
`too_many_lines`), `recorded_answers` (`:1269`, `too_many_lines`), and
`append_authorization_answer` (`fn` at `:1404`, its `#[allow]` at `:1400`,
`too_many_arguments`, six parameters).

That third suppression is **inert**. There is no `clippy.toml` anywhere in the
workspace, so the lint keeps its default threshold of seven, and six parameters
never fire it — `OrderingGraph::apply_resolution` (`edge/graph.rs:401`) carries
seven under the same deny-pedantic configuration with no suppression at all.
Deleting it is free and changes nothing. The projection type below is therefore a
design requirement in its own right, not lint relief, and gate 3 is written to
say so.

Phase 2 gave this file two more owners than the original split anticipated:
`CompleteBoardReport` and `ReservationLifecycleReport`, plus
`envelope_presentation` and `reservation_lifecycle_presentation`, which render
the complete report as presentation blocks. Each report type and its rendering
belong together in one module; do not leave the report types in the root while
their presentation moves.

Split along row assembly, visibility and omission policy, the
answer/disposition rendering, the report-and-presentation cluster, and a fifth
the original split missed: the alert, audit and git-cost surface. That fifth
cluster is `AvailableForcedPermit` (`:449`), `BypassAuditEntry` (`:459`),
`OutstandingIncursion` (`:501`), `BoardAlert` (`:535`), `BoardBranchRefStatus`
(`:566`), `BoardRetentionRefStatus` (`:583`) and `BoardGitCost` (`:617`), with
`outstanding_incursion_detail` (`:958`), `board_alert_detail` (`:982`),
`available_forced_permits` (`:1468`), `bypass_audit` (`:1487`),
`incursion_sections` (`:1578`), `board_alerts` (`:1631`) and `board_git_cost`
(`:1756`) — roughly 550 lines that belong to none of the other four. It gets
`board/alerts.rs`. Without it gate 1 is unreachable: this phase, unlike phase 9,
carries no clause licensing a different boundary, so a seat with nowhere to put
these would have to leave them in the root.
`append_authorization_answer` sits in the answer-rendering cluster. Its six
parameters are **not** one thing: `answers: &mut Vec<RecordedAnswer>` is an
output sink the function appends to, and `resolved_pairs` and `constraints` are
lookup projections it reads through. Only three describe the row being recorded.
Give that cluster a semantic projection type — name it
`RecordedAuthorizationConsequence` — carrying `reservation_id`, `authorization`,
and `acquisition`, and leave the sink and the two lookups as ordinary
parameters. The type says what the row is, where the parameter list only says how
many pieces it has.

`BoardModel` (`:103`) and its `build` constructor (`:736`) go to the **rows**
cluster: `build` assembles the model the row logic then reads, and no other
cluster names the type. Gate 2 turns on this, so it cannot be left implicit.

`board/tests.rs` is an existing sibling test module; move each test to sit with
the type it covers rather than leaving a catch-all. Like the root, it is written
by `impl` alone: the other seats copy the tests they need out of it at `HEAD`,
land them in their own modules, and never edit it, while `impl` empties it as
part of its own pass. No seat waits for another.

**Files:**
- `crates/cargo-berth/src/board/mod.rs`
- `crates/cargo-berth/src/board/tests.rs`
- `crates/cargo-berth/src/board/rows.rs`
- `crates/cargo-berth/src/board/visibility.rs`
- `crates/cargo-berth/src/board/answers.rs`
- `crates/cargo-berth/src/board/report.rs`
- `crates/cargo-berth/src/board/alerts.rs`

**Seats:** 3 writers + 0 testers — five clusters split cleanly, and
`board/tests.rs` is redistributed rather than owned by a test lane.
- `impl` — `board/mod.rs`, `tests.rs`, and `rows.rs`; hub: `board/mod.rs` and
  `tests.rs`, both of which `impl` alone writes
- `test` — opens as `impl`; `visibility.rs` and `answers.rs`
- `review` — opens as `impl`; `report.rs` and `alerts.rs`

**No seat writes another seat's file.** The pre-edit hook claims paths per
session, so an edit into a peer's claimed file is blocked rather than merged —
which makes a partition where three seats each delete their own cluster from one
root unexecutable. `impl` therefore owns the **entire** root transformation:
every `mod` declaration, every re-export, and every deletion. The other seats
create only their own submodule files, reading the code they move from the root
at `HEAD` (`git show HEAD:<root path>`) rather than from the file `impl` is
rewriting. Nothing serializes and no seat waits for a skeleton, because no seat
outside `impl` ever touches the root. Phase 7 paid for the older arrangement:
two seats converting callers of the same item at once produced a red where each
seat's own files were clean and neither error was its own.

**Acceptance gate:**
1. `board/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. No `too_many_lines` or `too_many_arguments` suppression remains under
   `crates/cargo-berth/src/board/`.
3. The `too_many_arguments` allow is deleted, `lint` stays green, and the audit
   row's own inputs travel as one named type — `RecordedAuthorizationConsequence`
   — with the output sink and the two lookup projections left as parameters.
4. The existing suite passes unmodified, including
   `tests/board.rs::populated_board_presentation_carries_the_complete_board_report`
   and the git-cost assertions at `tests/board.rs:2189` and `:3126`, which are
   what actually pin the wire surface the alerts cluster carries.
5. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 4 made `envelope_presentation` route
its empty case through `NonEmptyRenderedBlocks::try_from` to
`EnvelopePresentation::NothingToShow` rather than returning an empty vector, so
that conversion travels with the report cluster when it moves. Phase 1 rendered
the ambiguity outcome in
top-level `output.rs`, not in `board/mod.rs`, so no phase-1 rendering moves here.
Phase 2 added `CompleteBoardReport`, `ReservationLifecycleReport`,
`envelope_presentation`, and `reservation_lifecycle_presentation` to this file;
they are the reason it no longer fits in a shared phase with `gate/`. Phase 7 placed reservation-id ordering with `ReservationId` in `ids.rs`;
`board/mod.rs` calls it and does not re-implement it. Two consequences for this
split:

- `CompleteBoardReport::reservation_ids()` (`:918`) now **returns**
  `WireOrderedReservationIds` and builds it at `:946`, so that signature moves
  with the report cluster.
- `BoardGitCost` (`:617`) and `board_git_cost` (`:1756`) carry
  `trunk_resolution_calls` onto the wire: it appears twice in
  `docs/cargo-berth/generated/output-contract.json`, reaches users as
  `board --json` → `payload.data.git_cost.trunk_resolution_calls`, and is frozen
  in `tests/fixtures/front_end_corpus.json`. Phase 8 moved where that count is
  produced (into `ObservedRepositoryFacts`) without changing its value; this
  phase moves where it is rendered. The read at `:1803` travels with
  `board_git_cost` into `alerts.rs`, and gate 4's git-cost assertions are what
  prove the wire field survived the move.
- `board/mod.rs:1181` still sorts `worktree_heads` inline by
  `worktree_id.to_string()`. That orders `WorktreeId`, not `ReservationId`, and
  phase 7 ruled it out of scope deliberately. Leave it as it is — it is not a
  missed consolidation, and retyping it would widen a type phase 7 scoped on
  purpose.

---

### Phase 13 — `gate/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `gate/mod.rs` declares submodules and re-exports and carries no logic.

**Spec:** 1,404 lines beside `install.rs` and `permit.rs`, with no suppression of
its own — a pure move. Split along reference-transaction evaluation, branch
rewrites and re-anchoring, forced-permit auditing, and the gate *decision*
machinery. Tests move with the type each one covers.

The decision cluster is the largest and the original split had no home for it:
`evaluate_integration` (`:691`), `evaluate_locked` (`:765`), `decide` (`:886`),
`blocking_reservations` (`:948`), `decide_hook` (`:977`), `decide_integration`
(`:1050`), `skipped_holds` (`:1123`), `skipped_set_covers` (`:1154`) and
`GatePurpose` (`:1178`) — roughly 530 of the 1,404 lines. `decide` is reached
from the reference-transaction path (`:438`→`:831`), from `evaluate_integration`
(`:715`) and from `commit_forced_permit_audits` (`:661`), so it belongs to none
of the other three and cannot be folded into one of them without giving that
module two owners. It gets `gate/decision.rs`. The root's inline `mod tests` is
28 lines (`:1376`-`:1404`) and moves with the types it covers.

**Files:**
- `crates/cargo-berth/src/gate/mod.rs`
- `crates/cargo-berth/src/gate/reference_transaction.rs`
- `crates/cargo-berth/src/gate/rewrite.rs`
- `crates/cargo-berth/src/gate/audit.rs`
- `crates/cargo-berth/src/gate/decision.rs`

**Seats:** 3 writers + 0 testers — a pure move along four independent
boundaries, tests travelling with their types. The decision cluster gets its own
seat because it is the largest and every other cluster calls into it.
- `impl` — `gate/mod.rs` and `reference_transaction.rs`; hub: `gate/mod.rs` (the
  root, which `impl` alone writes)
- `test` — opens as `impl`; `decision.rs`
- `review` — opens as `impl`; `rewrite.rs` and `audit.rs`

**No seat writes another seat's file.** The pre-edit hook claims paths per
session, so an edit into a peer's claimed file is blocked rather than merged —
which makes a partition where three seats each delete their own cluster from one
root unexecutable. `impl` therefore owns the **entire** root transformation:
every `mod` declaration, every re-export, and every deletion. The other seats
create only their own submodule files, reading the code they move from the root
at `HEAD` (`git show HEAD:<root path>`) rather than from the file `impl` is
rewriting. Nothing serializes and no seat waits for a skeleton, because no seat
outside `impl` ever touches the root. Phase 7 paid for the older arrangement:
two seats converting callers of the same item at once produced a red where each
seat's own files were clean and neither error was its own.

**Acceptance gate:**
1. `gate/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. No suppression is added anywhere under `crates/cargo-berth/src/gate/`.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 7 placed reservation-id ordering with
`ReservationId` in `ids.rs`; `gate/mod.rs` calls it at `:953`, inside
`blocking_reservations` (`:948`) — which means the call lands in `decision.rs`,
and it is that seat's constraint, not the hub owner's. `gate/mod.rs` does not
re-implement it. That call is
`WireOrderedReservationIds::sorted_and_deduplicated`, not a plain sort — the
site had a pre-existing `dedup` and phase 7 kept it. Move the call intact; an
extracted helper that drops the deduplication changes what the gate reports. `gate/permit.rs` carries an `#[allow]` at `:473`, but it is
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

`ResolveArguments.why` (`crates/cargo-berth/src/cli.rs:647`) is a bare
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
reason string, which this plan's binding constraint forbids. **Delete the arm.**
All seven `uuid_identifier!` invocations (`:150-156`) select the plain arm, so
the `(future $name:ident)` arm at `:129` is never expanded, its suppression
compiles to nothing, and every `new()` in the crate already has real consumers —
"give the constructor a real consumer" is not an option that exists here.
Deleting an arm nothing invokes is precisely gate 2's "no speculative allows".
It is a multi-line attribute: a single-line `rg 'cfg_attr.*expect'` does not
match it.

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
1. Within this phase's three files — `cli.rs`, `ids.rs`, and `tests/board.rs` —
   a sweep covering both `#[allow]`/`#[expect]` and `cfg_attr`-wrapped forms shows
   no `too_many_lines`, `too_many_arguments`, `dead_code`,
   `needless_pass_by_value`, or `struct_excessive_bools` suppression.
2. `review` additionally runs the same sweep crate-wide, as a verification rather
   than as work. A survivor outside the three files above is the defect of the
   module phase that owned it — name that phase and stop; do not repair it here.
   The gate is scoped this way deliberately: phase 7 wrote a crate-wide sweep as a
   gate while seating by file, and instances of the swept concern turned up in a
   peer's file and in files no seat held, so no seat could satisfy the gate from
   inside its own boundary.
3. Every surviving allow names only pre-authorized test lints, and each one's
   module actually uses the lint's pattern — no speculative allows.
4. `CommandLineRoute::Resolve.arguments()` still builds a runnable resolve
   command line, and the three `cli.rs` route tests phase 6 added still pass
   unmodified.
5. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.
6. `bash ~/.claude/scripts/delegate/verify.sh final` passes, and
   `~/.claude/scripts/lint/lint mend`, `lint clippy --workspace`, and `lint doc`
   are all clean.

**Constraints from prior phases:** phase 6 added three unit tests to
`crates/cargo-berth/src/cli.rs` that this phase must keep compiling and true:
`only_the_hook_routes_answer_a_protocol_instead_of_an_envelope`,
`every_command_line_route_answers_through_the_output_ownership_it_declares`
(`:2594`), and the `ALL: [Self; 16]` route table they iterate. The resolve route
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
