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

### Phase 9 — `git/mod.rs` becomes a table of contents · status: done

#### As-built

`git/mod.rs` holds module documentation, `mod` declarations, and one `pub(crate) use`
block; nothing else. Every git query and the types describing its answers live in a
sibling named for the git concept it owns, and each module's tests moved with it.
`commit_target_reachability` is split into five helpers with three new state types —
`SoleCommitTarget`, `UnusableCommitTarget`, `TargetHistoryRead` — that name states
rather than representations, with control flow preserved exactly, including the
target-history join happening before the candidate join. `refs::name` is now
`reservation_retention_ref_name`; `apply_transaction` and `ReservationRetentionRef`
narrowed from `pub(super)` to private. No `#[allow]`, `#[expect]`, or `reason` string
exists anywhere under `git/`, and the existing suite passes unmodified: every name
outside `git/` and all observable behavior are unchanged.

**Files:**
- `crates/cargo-berth/src/git/mod.rs` — module declarations and the crate-facing re-export block
- `crates/cargo-berth/src/git/error.rs` — `GitError`, the single failure type every git query returns
- `crates/cargo-berth/src/git/object.rs` — object identity and commit availability
- `crates/cargo-berth/src/git/reachability.rs` — one target commit, a batch of candidates, a typed answer each
- `crates/cargo-berth/src/git/paths.rs` — which paths a batch of commits touched, returned raw for parsing
- `crates/cargo-berth/src/git/patch.rs` — scoped patch comparison and rebased-phase anchor location
- `crates/cargo-berth/src/git/conflict.rs` — merge-conflict coverage classification
- `crates/cargo-berth/src/git/refs.rs` — reservation retention refs
- `crates/cargo-berth/src/git/discovery.rs` — repository root and shared administrative directory
- `crates/cargo-berth/src/git/fixture.rs` — the shared `#[cfg(test)]` `PatchEquivalenceFixture`

**Gotchas:**
- A `pub(crate)` re-export the crate never names is an unused import and fails
  `-D warnings`, and no suppression may silence it. The remedy is an explicit type
  annotation at the call site, which makes the re-export used and documents the
  binding; `verb/release.rs` and `drift/provenance.rs` each carry one for this reason.
- A module named for a git concept owns only that concept. Two path-attribution
  readers living in the reachability module made its documentation false, which is
  why `paths.rs` exists.
- A root file's inline `#[cfg(test)]` module dies with the root. Shared test support
  becomes its own module rather than being duplicated across the new siblings.
- `gate::committed_hook_persists_one_scoped_patch_evaluation_record` is bounded by a
  10-second wall clock: it fails under a contended tree and passes in 1.7s on a quiet
  one. Its own message names the deadline, which is what tells contention from a defect.

**Ruled out:**
- Widening a module to `pub(crate) mod` to make a moved type nameable — it compiles,
  but lets callers name items around the root and changes the paths callers write.
- Rewording the reachability module's documentation to cover the two path readers
  instead of moving them out.

### Phase 10 — `reservation/mod.rs` becomes a table of contents · status: done

#### As-built

`reservation/mod.rs` is 62 lines: module documentation, nine private `mod`
declarations, and one `pub(crate) use` block of 42 entries. No module is
`pub(crate) mod`, so no caller can name an item around the root and every path
shape is unchanged. Both `too_many_lines` suppressions are gone, removed by
restructuring rather than by moving the code: `RetainedReservationSet::apply` is
a 30-line dispatch over all eighteen `JournalOperation` variants with no
catch-all, routing to four named family helpers plus four variants that are
no-ops for this projection; `ReservationConflict`'s `Display::fmt` delegates to
`write_holder_fault` and `write_active_holder_fault`. The suite is 534 tests,
the same count as before the split.

**Files:**
- `reservation/mod.rs` — declarations and re-exports only
- `reservation/retention.rs` — `RetainedReservationSet` and both its impls, the incursion types, the inline test module
- `reservation/record.rs` — `Reservation` and its impl, `ReservationFreshness`, `ReservationHolderActivity`, `ReservationEvidenceState`, `ReservationLifecycleSnapshot`
- `reservation/scoped_patch_evaluation.rs` — the scoped-patch evaluation cluster
- `reservation/replay.rs` — `ReplayedClaim`, `ReservationReplayError` with its `Display` and `Error` impls
- `reservation/partition.rs` — `DriftBlockingCoverage`, `WidenScopeBinding`, `AuthorizedEditingIdentity`, `reservations_authorize_scope`
- `reservation/conflict.rs` — `ReservationConflict` and its impl

`constants.rs`, `evidence.rs`, and `lifecycle.rs` were already siblings and are
unchanged.

**Binds later work:** every widening needs a verified cross-module caller —
`RetainedReservationSet`'s two fields were widened to `pub(super)` during the
split with no reader outside the file, and all three review lenses caught it
independently; the remaining module phases carry the rule, and an existing
accessor beats a new widening. Splitting an exhaustive match into family helpers
narrows what the compiler checks: the outer dispatch stays exhaustive, but each
helper's `_ => Ok(())` swallows a mis-routed variant, so the one remaining phase
that splits a dispatch may not nest a second catch-all beneath the existing one.
Eligibility is now `reservation/record.rs:158` and `:171`,
`has_other_active_reservation` is `reservation/retention.rs:487`, and
`ReservationReplayError::DuplicateIncursionIncident` is `reservation/replay.rs:48`.

**Gotchas:** a `verify.sh lint` run that exits 0 in under a second with no
`Checking <package>` line is a cache hit and proves nothing — `touch` does not
bust it, `cargo clean -p <package>` does.
`gate::batched_attribution_benchmark_covers_short_and_long_ranges` is a
wall-clock comparison that reads as a defect under load: it failed at 67.9s
during a loaded full-suite run and passed alone at 21.7s on the same tree.
`authorizes` is the one moved body that is not byte-identical — it reads
`reservations.iter()` where the root read `reservations.reservations.iter()` —
and that is what made the visibility defect fixable.

**Ruled out:** `pub(crate) mod` as the widening for cross-module access, which
compiles but lets callers name items around the root and changes the path shape;
suppressing the unused-import error a `pub(crate)` re-export the crate never
names produces, where the remedy is an explicit type annotation at the call
site; sharing `TRUNK_OID` between the two test modules that now define it, since
a fixture local to its own test module is correct and sharing it would mean
opening a private test module; narrowing the four family helpers so the compiler
checks the inner match, which is real work with its own design decision and
belongs to the backlog rather than to a behavior-preserving phase.

### Phase 11 — `ledger/mod.rs` becomes a table of contents · status: done

#### As-built

`ledger/mod.rs` is an index: module documentation, twelve private `mod`
declarations, a `#[cfg(test)] mod test_support;` carrying the pre-authorized
`#[allow(clippy::expect_used, reason = ...)]` in outer position, and one
`pub(crate) use` block of 62 entries. It holds no logic. The former 2,559-line
root's contents live in eight sibling files, moved without behavior change: a
normalized comparison of every original line against the new files leaves only
declared visibility prefixes, test attributes, rustfmt re-wrapping, and two path
rewrites the new module depth forced. All thirteen inline tests and their helpers
survive, each body byte-identical apart from those rewrites.

Eleven items became `pub(super)`, each with a verified caller in a different
file; nothing reaches past `pub(crate)`. Four more that could have been widened
stayed private because their only readers live in the same file.
`ledger::test_support` supplies `scratch_repository` to three destination test
modules — `handle.rs`, `worktree_context.rs` and `authorization.rs`.
`identity.rs` builds its own `tempdir()` and does not use it.

**Files:**
- `crates/cargo-berth/src/ledger/mod.rs` — the index: declarations and re-exports only
- `crates/cargo-berth/src/ledger/authorization.rs` — `EditAuthorization`, its resolution, and `EnvironmentCoordinationRunSelection`
- `crates/cargo-berth/src/ledger/identity.rs` — repository and worktree identity readers
- `crates/cargo-berth/src/ledger/path.rs` — absolute-path normalization and ancestor canonicalization
- `crates/cargo-berth/src/ledger/test_support.rs` — `scratch_repository`, shared by three test modules
- `crates/cargo-berth/src/ledger/handle.rs` — `Ledger`, its transactions, and eight of the moved tests
- `crates/cargo-berth/src/ledger/error.rs` — the ledger error family
- `crates/cargo-berth/src/ledger/coordination_run_marker.rs` — marker detachment, retirement and sweep
- `crates/cargo-berth/src/ledger/worktree_context.rs` — `WorktreeContext`, worktree availability, and the git-layout readers

**Binds later work:** the remaining table-of-contents splits inherit four rules
from this one. A shared `#[cfg(test)] mod test_support;` on the root replaces one
private copy of a helper per destination test module. The pre-authorized test
allow goes in outer form on that declaration, never as an inner `#![allow(...)]`
inside the helper file — so a root that is otherwise only declarations still
carries one attribute, and the suppression phase's crate-wide sweep must
recognize it. Visibility is a three-rung ladder — private, `pub(super)` for a
verified reader in a sibling file, `pub(crate)` plus a root re-export only for an
item the crate names outside the module — chosen from where the callers actually
are. Every cluster boundary is posted to the board before any seat deletes from
`HEAD`.

**Gotchas:**
- A `pub(crate)` re-export of an item the crate does not name outside the module
  is an unused import, and `-D warnings` makes it a hard error. Six items here
  are correctly absent from the root's list.
- A `pub(crate)` type in a private module is unnameable outside it unless the
  root re-exports it. Four are not, so a binding whose type is one of them cannot
  be annotated: `worktree_identity` is re-exported while `WorktreeIdentity` is
  not.
- `batched_attribution_benchmark_covers_short_and_long_ranges` compares two
  measured durations and fails under compilation load rather than on a
  regression.

**Ruled out:**
- Placing `EnvironmentCoordinationRunSelection` in the marker module — it reads
  an environment variable and touches no marker file, and the placement would
  have widened both the enum and its constructor for a caller in a third file.
- Placing the marker-retirement test with the marker module — it calls a private
  method of `WorktreeContext`, which the placement would have widened for a
  test-only reader.
- One private copy of `scratch_repository` per destination test module.

### Phase 12 — `board/mod.rs` becomes a table of contents · status: done

#### As-built

`board/mod.rs` is an index: one line of module documentation, five private `mod`
declarations, `pub(crate) mod tui;`, a `#[cfg(test)] mod test_support;`, and four
`pub(crate) use` entries. It holds no logic and declares no type. The former
1,900-line root's contents live in six sibling files, moved without behavior
change; the deleted `board/tests.rs` redistributed its five tests to the code
each one covers — three to `rows`, one to `alerts`, one to `answers`.

The module's whole external surface is `reservation_lifecycle_presentation`,
`reservation_lifecycle_snapshot`, `BoardModel`, `LiveIncursionMembership`, and
`tui`. That is exactly what `output.rs` and `verb/board.rs` name. `BoardError` is
`pub(crate)` inside a private module with no root re-export, so nothing outside
`board/` can name it — which is correct, because nothing does.

Unlike `ledger::test_support`, `board::test_support` calls no `.expect()`, so its
declaration carries no suppression and the root is attribute-free.

**Files:**
- `crates/cargo-berth/src/board/mod.rs` — the index: declarations and re-exports only
- `crates/cargo-berth/src/board/rows.rs` — `BoardModel`, `LiveIncursionMembership`, `DeclaredOrderingConstraints`, and the row projection
- `crates/cargo-berth/src/board/alerts.rs` — coordination notices and their ordering
- `crates/cargo-berth/src/board/answers.rs` — `RecordedAnswer` and the recorded-answer sections
- `crates/cargo-berth/src/board/report.rs` — `reservation_lifecycle_presentation` and `reservation_lifecycle_snapshot`
- `crates/cargo-berth/src/board/error.rs` — `BoardError`
- `crates/cargo-berth/src/board/test_support.rs` — the shared test helper, `#[cfg(test)]` only

**Binds later work:** making an enum `pub(super)` widens every type its variants
name. Variant fields are always as visible as their enum, and `private_interfaces`
is a hard error under `-D warnings`, so a `pub(super)` enum carrying a private
type does not compile. Struct fields are exempt — they default to private, so a
narrow type behind a private struct field leaks nothing. These widenings are
language-forced, not discretionary: the rule requiring a named cross-module
reader before widening does not apply to them, because the compiler names the
readers and the only alternative is leaving the enum private. Two of this split's
files hit it at once — ten errors in one and seven in another — and each was
resolved against a real compile rather than by reading the code.

**Gotchas:**
- A relocated raw string must keep its `"#` terminator at column 0. Re-indenting
  it to match the surrounding block appends a newline and four spaces to the
  value; the compile stays clean and the test fails on content.
- `board::test_support` needs no test allow because it never calls `.expect()`.
  The inherited rule is that the suppression follows the helper's actual calls,
  not the presence of a shared helper.

**Ruled out:**
- Re-exporting `BoardError` from the crate root so it can be named outside
  `board/` — no such consumer exists, and a future one is a re-export, never a
  wider type.
- Widening `batched_attribution_benchmark_covers_short_and_long_ranges`'s timing
  margin to stop its load-dependent failures.

### Phase 13 — `gate/mod.rs` becomes a table of contents · status: done

#### As-built

`crates/cargo-berth/src/gate/mod.rs` is a table of contents: seven `mod`
declarations and eighteen `pub(crate) use` re-exports, and nothing else. Its
former body moved verbatim into five new siblings beside the unchanged
`install.rs` and `permit.rs`. The move is behavior-preserving by construction —
an ordered per-item body diff matched all 55 top-level items at the prior commit
against all 55 afterwards, by normalized signature, and the only difference
across all of them is rustfmt re-wrapping `GatePurpose::identity_validation` onto
three lines. No `crate::` import was added or removed anywhere.

**Files:**
- `crates/cargo-berth/src/gate/mod.rs` — module declarations and the crate-facing
  re-export list for the gate.
- `crates/cargo-berth/src/gate/decision.rs` — `GateDecision`, `GateResult`,
  `IntegrationRequest`, `IntegrationViolation`, `GatePurpose`, and
  `evaluate_integration`.
- `crates/cargo-berth/src/gate/reference_transaction.rs` — the git
  reference-transaction protocol: parsing, phase and presence types,
  `ManagedTrunkDeletion`, the issuing-directory environment variable, and
  `evaluate_reference_transaction`.
- `crates/cargo-berth/src/gate/rewrite.rs` — `BranchRewrite` and
  `branch_rewrites`, module-internal.
- `crates/cargo-berth/src/gate/error.rs` — `GateError`.
- `crates/cargo-berth/src/gate/audit.rs` — the gate's audit record construction.

**Binds later work:** the only suppression anywhere under
`crates/cargo-berth/src/gate/` is `permit.rs:473` — `clippy::expect_used` on
`mod tests` — which is pre-authorized boilerplate. Nothing under `gate/` is an
item for the suppression-removal phase.

**Gotchas:**
- Enum visibility is transitive to variant field types: widening an enum to
  `pub(super)` forces every type its variants name to at least `pub(super)`,
  because `private_interfaces` is a hard error under `-D warnings`. Struct fields
  are exempt. These widenings are language-forced, not discretionary.
- A widening check that only looks for other files naming an item produces false
  positives. An item nothing else names is still forced wider by appearing in a
  widened function's signature — `BranchRewrite` is returned by
  `pub(super) fn branch_rewrites`. Check signatures beside readers.
- A clippy or test exit 0 with no `Checking`/`Compiling`/`Documenting
  cargo-berth` line in the output is a cache hit, not a pass. `touch` does not
  bust it; `cargo clean -p cargo-berth` does.
- `GateError`'s `Display` arms are asserted nowhere. This predates the split.

**Ruled out:** splitting `install.rs` or `permit.rs`, which were already
single-concern siblings and never part of the hub body; and any concern that
narrowing visibility could put an integration test out of reach — `cargo-berth`
is a binary crate with no `[lib]`, and every integration suite drives the
compiled binary through `std::process::Command`.

### Phase 14 — Remove the remaining suppressions · status: todo

#### Work Order

**Goal:** No suppression remains anywhere in `crates/cargo-berth/src/`, except the
pre-authorized test-module boilerplate — `clippy::expect_used`, and
`clippy::panic` where the module uses `panic!`.

**Spec:** Four sites survive the earlier phases, in three shapes. All four, and
every line reference below, are confirmed against the tree as it stands after
phase 13.

`crates/cargo-berth/tests/board.rs` holds two: a `too_many_lines` at `:881` on
`release_dispositions_remain_resolved_when_trunk_rewrites` (`:885`, running to
`:1006`) and a `needless_pass_by_value` at `:4385` on
`append_journal_operation_with_actor`, which has four call sites. The test splits
into its arrangement and its per-disposition assertions; the helper takes its
payload by reference.

`crates/cargo-berth/src/cli.rs:585` suppresses `struct_excessive_bools` on
`ResolveArguments`. The struct holds exactly four bools — `every_incursion`,
`recovered`, `abandon`, `retire_orphan` — against clippy's default
`max-struct-bools = 3`, which no workspace configuration overrides. **Moving all
four into one new boundary type re-trips the identical lint on that type.** Split
them instead along the partition `ResolveDecision` already names in its own two
variants:

- an incursion-answer selection carrying `incursion: Option<IncursionIncidentId>`
  and `every_incursion: bool`, converting into `IncursionAnswerScope`;
- a reservation-recovery selection carrying `recovered`, `abandon`,
  `retire_orphan` and their value-bearing partners `integrated_as` and `why`,
  converting into `ReservationRecoveryDecision`.

Each is a `#[command(flatten)]` group holding at most three bools, so the count
falls under the threshold in both rather than being excused, and each converts
immediately at the Clap boundary so nothing optional reaches the verb.

**`why` keeps its `Option<String>`, confined to the boundary type. Do not
introduce a `ResolveJustification` enum.** `crates/cargo-berth/src/cli.rs:1292`–
`:1300` already parses `why` through `AbandonmentReason` and
`OrphanRetirementReason` (`crates/cargo-berth/src/reservation/lifecycle.rs:300`,
`:306`) — non-empty-by-construction newtypes with fallible `FromStr`, each
carried by the `ReservationRecoveryDecision` variant it justifies. A `String`
therefore never reaches `ResolveDecision` today, and a type distinguishing a
stated justification from an unstated one would be less specific than what
exists while modelling a state `requires = WHY_ARGUMENT` makes unreachable.
Convert the raw optional straight into those two existing reasons, exactly as
the current code does.

**The user-visible flag surface is frozen; only the parsed representation
changes.** The flag spellings, their `ArgGroup` membership
(`RESOLVE_DISPOSITION_GROUP`, `RESOLVE_REASONED_DISPOSITION_GROUP`), and their
`requires` relationships all survive intact. Nine rendered engine strings
hard-code the spellings — `crates/cargo-berth/src/output.rs:2980`, `:2984`,
`:2989`, `:2990`, `:3471`, `:3528`, `:3531`, `:3961` — and
`crates/cargo-berth/tests/presentation.rs:168`, `:173`, `:498` and `:501` assert
the literal text `cargo-berth resolve {id} --integrated-as` and
`cargo-berth resolve {id} --abandon --why`. Further invocations live in
`crates/cargo-berth/tests/answers.rs:1765` and
`crates/cargo-berth/tests/lifecycle.rs:166`, and the frozen fixture
`crates/cargo-berth/tests/fixtures/front_end_corpus.json` carries thirty
`resolve` occurrences. Renaming or regrouping a flag rewrites that fixture, so
do not.

**The refusal the parser renders is part of this phase's surface.** The `_ =>`
arm at `crates/cargo-berth/src/cli.rs:1301` returns the user-facing text
`choose exactly one resolution disposition and provide --why only for --abandon
or --retire-orphan`, and the `requires` edges produce clap's own rejections.
Replacing the flag set rewrites or deletes that arm. Preserve the refusal text
and every clap rejection path, or state the replacement text and cover it with a
test — this is the one externally observable behavior the phase can change, and
it does not get to change silently.

**The resolve route is a wire fact, not only a parser fact.** Phase 6 added
`CommandLineRoute::Resolve.arguments()` (`crates/cargo-berth/src/cli.rs:2070`),
which builds the literal argv `resolve <id> --recovered --json` for the recovery
command the engine prints. The route table is part of this phase's surface and
its acceptance gate.

`crates/cargo-berth/src/ids.rs:132` carries a
`cfg_attr(not(test), expect(dead_code, …))` on the `uuid_identifier!` macro's
`future` constructor arm — an unused-outside-tests suppression that authors a
reason string, which this plan's binding constraint forbids. **Delete the arm.**
`macro_rules! uuid_identifier` is declared **twice** in this file: `:22` defines
the type, and `:128` shadows it to add `new()`. Seven invocations follow each —
`:120`-`:126` under the first, `:150`-`:156` under the second — so a sweep for
`uuid_identifier!` finds fourteen sites, not seven, and none of the fourteen is
a `uuid_identifier!(future …)`. The `(future $name:ident)` arm at `:129` and its
`cfg_attr` at `:132` belong to the second declaration alone. All seven
invocations under it (`:150`-`:156`) select the plain arm, so the `future` arm is
provably never expanded, its suppression compiles to nothing, and every `new()`
in the crate already has real consumers — "give the constructor a real consumer"
is not an option that exists here.
Deleting an arm nothing invokes is precisely gate 2's "no speculative allows".
It is a multi-line attribute: a single-line `rg 'cfg_attr.*expect'` does not
match it.

`crates/cargo-berth/src/ledger/journal.rs` is **no longer a site.** Its
`dead_code` suppression on the macro-generated `wire_name` is already gone — the
method is test-only and exercised — so nothing there remains for this phase.

Then sweep the whole crate and prove the claim: the only `#[allow]`/`#[expect]`
attributes left name `clippy::expect_used` or `clippy::panic` on a
`#[cfg(test)]` module or on a whole integration-test file, which
`~/rust/nate_style/rust/test-module-allow-boilerplate.md` pre-authorizes. A
`cfg_attr`-wrapped suppression counts; search for both spellings.

**Read but never written by this phase, since the flag surface is frozen:**
`crates/cargo-berth/src/output.rs`, `crates/cargo-berth/tests/presentation.rs`,
`crates/cargo-berth/tests/answers.rs`, `crates/cargo-berth/tests/lifecycle.rs`,
`crates/cargo-berth/tests/fixtures/front_end_corpus.json`, and
`docs/cargo-berth/generated/output-contract.json`.

**Files:**
- `crates/cargo-berth/tests/board.rs`
- `crates/cargo-berth/src/cli.rs`
- `crates/cargo-berth/src/ids.rs`

**Seats:** 2 writers + 1 tester — `impl` carries the whole `cli.rs` redesign plus
the frozen-surface obligation, `test` writes the two `tests/board.rs` sites, and
`review` opens on a one-line deletion, so it takes the verification lane that the
phase's one behavior-shaping change needs.
- `impl` — `cli.rs`
- `test` — `tests/board.rs` (both sites)
- `review` — opens as `test`; `ids.rs`, then owns the resolve-argv verification
  lane across `tests/presentation.rs`, `tests/answers.rs` and
  `tests/lifecycle.rs`, proving the rendered recovery commands still run
  verbatim against the rebuilt parser.

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
   module actually uses the lint's pattern — no speculative allows. The nine
   file-level inner allows named in the constraints below are expected hits, not
   findings.
4. `CommandLineRoute::Resolve.arguments()` still builds a runnable resolve
   command line, and the three `cli.rs` route tests phase 6 added still pass
   unmodified.
5. Every rendered recovery command still runs verbatim: `tests/presentation.rs`,
   `tests/answers.rs`, `tests/lifecycle.rs`, and the
   `tests/fixtures/front_end_corpus.json` corpus all pass **unmodified**. A diff
   touching any of them means a flag spelling moved, which this phase forbids.
6. The `_ =>` refusal text at `cli.rs:1301` is preserved verbatim, or its
   replacement is stated in the report and covered by a test.
7. `docs/cargo-berth/generated/output-contract.json` is unchanged. It carries
   `resolve --integrated-as <trunk-oid>` at three places sourced from an
   `output.rs` doc comment; the frozen flag surface means nothing regenerates.
   If it does change, a flag spelling moved and gate 5 has already failed.
8. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.
9. `bash ~/.claude/scripts/delegate/verify.sh final` passes, and
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
it is that phase's defect, not a new item here. Phase 13 added no suppression at
all: the only attribute anywhere under `src/gate/` is `permit.rs:473`
(`clippy::expect_used` on `mod tests`), which is pre-authorized boilerplate.
Every other `#[allow]` still in `crates/cargo-berth/src/` names
`clippy::expect_used` or `clippy::panic` on a `#[cfg(test)]` module and is
pre-authorized boilerplate. Phase 11 introduced a
second spelling of that boilerplate: `crates/cargo-berth/src/ledger/mod.rs`
carries `#[allow(clippy::expect_used, reason = ...)]` in **outer** position on its
`#[cfg(test)] mod test_support;` declaration, rather than as an inner
`#![allow(...)]` inside the module file. Gate 2's crate-wide sweep surfaces it in
a table-of-contents root that carries no other logic; it is pre-authorized in the
same sense as the rest, its module does call `.expect(`, and it is nobody's item.
A sweep that only looks inside `mod tests` bodies misses it. There is a third
spelling the sweep will also surface: nine integration-test files —
`tests/answers.rs`, `board.rs`, `drift.rs`, `edges.rs`, `gate.rs`, `ledger.rs`,
`lifecycle.rs`, `liveness.rs` and `overlap.rs`, including one of this phase's own
three files — each open with a file-level inner
`#![allow(clippy::expect_used, reason = …)]` at line 1. An integration-test file
is wholly a test module, so the inner form is the only form available to it;
all nine are pre-authorized and none is this phase's work. The sites named above
were never owned by an earlier phase and are this phase's own work.

## Gates

- Every phase: `verify.sh test cargo-berth` and `verify.sh lint cargo-berth`.
- Final: `verify.sh final`, plus `lint mend`, `lint clippy --workspace`, `lint doc`.
- No phase adds a suppression. No phase pushes.
