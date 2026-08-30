# cargo-berth structure and selection fixes

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Removes every
> `clippy::too_many_lines` suppression by splitting the function it guards,
> turns the five module roots that carry logic back into tables of contents,
> repairs the first-touch selection defect that lets replay order override an
> exact session reservation mapping, gives that repair a way out when the
> selection is ambiguous, and makes an installed front end notice a binary it no
> longer matches instead of misreporting it.

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
  `drift/`, `gate/`, `git/`, `ledger/`, `board/`, `scope/`, `session/`,
  `worktree/`, `answer/`, plus top-level `alert.rs`, `cli.rs`, `config.rs`,
  `constants.rs`, `coordination_identity.rs`, `exit.rs`, `ids.rs`, `output.rs`,
  `output_contract.rs`, `reconcile.rs`, `recovery.rs`.
  `crates/cargo-berth/tests/` holds the integration suites: `answers.rs`,
  `board.rs`, `drift.rs`, `edges.rs`, `gate.rs`, `ledger.rs`, `lifecycle.rs`,
  `liveness.rs`, `overlap.rs`.
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

### Phase 2 — Recovering from an ambiguous first touch, and consumers that survive a version change · status: todo

#### Work Order

**Goal:** A first touch that cannot choose between two reservations names the
command that resolves it, and an installed consumer built against an older
binary says so and names its repair instead of misreporting the response.

**Spec:** Phase 1 added the `AmbiguousActiveRunReservations` outcome: when no
usable harness-session mapping selects one active reservation and more than one
is eligible for the acting run and worktree, `check` exits `BlockedByOverlap`
carrying the candidate ids, appending nothing, widening nothing, and publishing
no mapping. That outcome is correct and observable, and nothing can act on it.
Two halves close it.

**(a) A selector that resolves the ambiguity.**
`OutputEnvelope::ambiguous_active_run_reservations`
(`crates/cargo-berth/src/output.rs:1907`) renders the candidate ids and names no
runnable action, and `Command::Check` (`crates/cargo-berth/src/cli.rs:206`)
takes `PathArguments`, which carries no reservation selector — so a caller
holding two ids has no way to say which one it means, and
`CheckRequest { declared_scopes }` (`cli.rs:827`) has nowhere to put the answer.
Introduce a `CheckArguments` that wraps the existing path arguments and an
optional reservation id, and carry the resolved choice into `CheckRequest` as
`CheckReservationSelection::{SessionMappingOrSingleActive, Explicit}`. The bare
`Option<ReservationId>` may exist only at the Clap boundary and converts
immediately into that state; it never reaches `CheckRequest` or below. An
explicit selection admits only a reservation the acting run and worktree already
hold — the same eligibility
`Reservation::is_active_for_coordination_run_and_worktree`
(`crates/cargo-berth/src/reservation/mod.rs:1847`) already enforces — republishes
the harness-session mapping onto it, and leaves the next ordinary check
succeeding against that reservation. The ambiguity message names that command.

**(b) Consumers that detect their own staleness.** The installed pre-edit hook
`~/.claude/scripts/berth/install/hooks/berth_pre_edit.sh` accepts exit 1 only as
`blocked_by_overlap` carrying a `check` payload with a non-empty `conflicts`
array, and `~/.claude/scripts/berth/claim_state.py` classifies the same closed
set of statuses by hand. Neither knows this outcome, so installing the current
binary turns a legitimate ambiguity into `check returned a malformed or
inconsistent blocked envelope` on every edit in a repository with two open
reservations: the edit is refused, which is safe, for a stated reason that is
false and that hides the candidate ids. The generated half of the installation
already regenerates from the binary on every install —
`generated/status_payload_tables.py` and `generated/envelope_validation.jq`,
staged, validated, and published with rollback by
`~/.claude/scripts/berth/install/install.sh` — but the hand-written classifiers
do not, and nothing anywhere detects the skew.

Close the class rather than this one status:

- The binary reports its output-contract version; `install.sh` stamps that
  version beside the generated assets; the hook compares the stamp against the
  binary on every run and, on mismatch, refuses with the repair command named
  and the mismatch stated. The hook already carries this shape for an unreadable
  `envelope_validation.jq`, where it sets a needs-repair installation state and
  says to repair the installation — generalize that path rather than adding a
  second one beside it.
- `install.sh` gains `--reset`, which removes the generated directory and the
  version stamp wholesale before regenerating, so an installation carrying
  assets from an older binary is repaired by reinstalling rather than by hand.
- Teach the pre-edit classifier and `claim_state.py` the ambiguity outcome, so a
  user who reaches it sees the candidates and the recovery command from (a).
- Exercise both hook routes against the current binary: the PreToolUse route
  through `berth_pre_edit.sh` and the PostToolUse route through
  `berth_post_bash.sh`. Report what each does rather than assuming either is
  already correct.

`docs/cargo-berth/json-contract.md` is the stable wire contract and describes
neither `ambiguous_active_run_reservations`, nor its
`first_touch_reservation_selection` payload, nor the contract-version field this
phase adds. Document all three there.

**Files:**
- `crates/cargo-berth/src/cli.rs`
- `crates/cargo-berth/src/verb/check.rs`
- `crates/cargo-berth/src/verb/claim.rs`
- `crates/cargo-berth/src/output.rs`
- `crates/cargo-berth/src/output_contract.rs`
- `crates/cargo-berth/src/reservation/mod.rs`
- `crates/cargo-berth/tests/overlap.rs`
- `docs/cargo-berth/generated/output-contract.json`
- `docs/cargo-berth/json-contract.md`

**Acceptance gate:**
1. A fixture with two eligible active reservations and no usable mapping proves
   that an explicit reservation selection republishes the mapping onto the named
   reservation and that the next ordinary check reports it rather than the
   ambiguity.
2. A fixture proves an explicit selection naming a reservation the acting run
   and worktree do not hold is refused, appends nothing, and publishes no
   mapping.
3. The pre-edit hook, run against a stamped older installation, refuses with the
   version mismatch stated and the repair command named; `install.sh --reset`
   clears and regenerates the assets, after which the same edit succeeds.
4. The pre-edit hook, run against a matching installation, renders the ambiguity
   with its candidates and the recovery command, and
   `~/.claude/scripts/berth/tests/test_hook_rendering.py` covers that rendering
   and the PostToolUse route.
5. `generated_artifacts_are_reproducible_from_the_checked_in_contract` passes
   after regenerating with the documented command.
6. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 1 shipped `FirstTouchReservationSelection`
in `crates/cargo-berth/src/verb/claim.rs` — a private, invocation-local decision
type with `SessionMappedReservation`, `SingleActiveRunReservation`,
`NoActiveRunReservation`, and `AmbiguousActiveRunReservations {
candidate_reservation_ids }`. The harness-session mapping that decides the first
state is read inside the ledger transaction; the acting coordination run is
resolved before that lock (`verb/claim.rs:557-562`), and that is safe only
because eligibility requires `actor.run == coordination_run_id`, so the unmapped
fallback can only ever select the acting run's own reservation. Preserve the wire
names `ambiguous_active_run_reservations` and `first_touch_reservation_selection`
byte-for-byte, and regenerate `docs/cargo-berth/generated/output-contract.json`
with the command in the Delegation Context rather than editing it by hand.

**Hook layer:** the pre-edit hook, `claim_state.py`, `install.sh`, and the hook
rendering tests live outside this repository under
`~/.claude/scripts/berth/`. They are in scope for this phase and the checkpoint
commit cannot carry them; state what changed there in the summary so the change
is visible without a diff.

---

### Phase 3 — One home for run eligibility and reservation-id ordering · status: todo

#### Work Order

**Goal:** The active-for-this-run predicate and the deterministic ordering of
reservation ids each have exactly one implementation, placed so the later module
phases move code without carrying a duplicate with them.

**Spec:** Two idioms are spread across the crate, and the module phases that
follow cannot consolidate them because neither belongs to a single module root.

The eligibility predicate exists as
`Reservation::is_active_for_coordination_run_and_worktree`
(`crates/cargo-berth/src/reservation/mod.rs:1847`), which phase 1 added and
`verb/claim.rs:383,403` calls, while the same `actor.run == …
&& actor.worktree == …` comparison is still written out inline at
`reservation/mod.rs:818-819`, `:831-832`, and `:998`. Route every site that means
"active for this run and worktree" through the method. Where a site means
something narrower, say so at that site rather than widening the method.

Reservation-id ordering by rendered string appears five times:
`verb/claim.rs:414` (`sort_by_cached_key`), `drift/ordering.rs:12`,
`output.rs:2902`, `board/mod.rs:775`, and `gate/mod.rs:961`. `drift::ordering` is
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

**Constraints from prior phases:** phase 2 may have added an explicit reservation
selection that reuses the eligibility predicate; it uses the single home this
phase establishes. This consolidation runs before the module phases deliberately:
the ordering must not land in `reservation/mod.rs`, whose own phase reduces it to
a table of contents, and the predicate stays with the `Reservation` type so it
moves with that type when that phase runs.

---

### Phase 4 — Split the generated-contract builders · status: todo

#### Work Order

**Goal:** `output_contract.rs` carries no `too_many_lines`, `dead_code`, or
`needless_pass_by_value` suppression, and the generated artifact is byte-identical
to the one checked in when this phase starts.

**Spec:** Five functions and two other suppressions live in this file:

| Site | Function | Body |
| --- | --- | --- |
| `:276` | `outcome_rules` | 292 lines |
| `:785` | `generated_fixtures` | 264 lines |
| `:1081` | `envelope` (`needless_pass_by_value`) | — |
| `:1300` | `render_python_tables` | 409 lines |
| `:1832` | `render_jq_validator` | 126 lines |
| `:2044` | `generated_python_exports_wire_name_discriminators` | 155 lines |
| `:2417` | (`dead_code` on a test-only type) | — |

`outcome_rules` and `generated_fixtures` are declarative tables; split them by
the domain each group of rows describes — one function per status family — and
have the parent concatenate. `render_python_tables` and `render_jq_validator`
are sequential template emitters; give each emitted section its own function
taking the writer, so the parent reads as the document's outline. The
`needless_pass_by_value` site takes an owned payload it only reads: borrow it,
and adjust the fixture call sites. The `dead_code` type exists only to exercise
schema generation; give it a use in the assertion that already covers it, or
delete it if the assertion does not need it.

**Files:**
- `crates/cargo-berth/src/output_contract.rs`
- `docs/cargo-berth/generated/output-contract.json`

**Acceptance gate:**
1. No `#[allow]` or `#[expect]` remains in `output_contract.rs` other than the
   pre-authorized `clippy::expect_used` test-module boilerplate at `:1960`.
2. `generated_artifacts_are_reproducible_from_the_checked_in_contract` passes
   **without** the regenerate environment variable, proving the artifact did not
   move.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 1 added the
`ambiguous_active_run_reservations` status, the
`first_touch_reservation_selection` payload, and their generated rows and
fixtures; phase 2 adds the contract-version field and a reservation selector on
`check`. Both edited this file and regenerated the artifact, so every anchor in
the table above shifts — confirm each site in the current file before splitting
it. Preserve those wire names byte-for-byte: this phase splits functions and
changes no emitted bytes.

---

### Phase 5 — Split the reconciliation planners · status: todo

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

**Acceptance gate:**
1. No `too_many_lines` suppression remains in `reconcile.rs`.
2. The existing reconciliation tests pass unmodified — this phase changes no
   behavior.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** none — no earlier phase touched this file.

---

### Phase 6 — `git/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `git/mod.rs` declares submodules and re-exports, and carries no logic
and no `too_many_lines` suppression.

**Spec:** The root holds roughly 3,100 lines past its declarations — the largest
offender in the crate — beside existing `command.rs`, `constants.rs`, and
`refs.rs` submodules. Split by type ownership, not by code category: each new
submodule is named after the anchor type or the git concept it owns, and its
tests move with it. Candidate boundaries visible today, to be confirmed against the
code rather than taken as given: reachability and ancestry queries (including
`commit_target_reachability` at `:1883`, 151 lines, which is also a
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

### Phase 7 — `reservation/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `reservation/mod.rs` declares submodules and re-exports, and carries no
`too_many_lines` suppression.

**Spec:** Roughly 2,500 lines past its declarations, beside existing
`constants.rs`, `evidence.rs`, and `lifecycle.rs`. Two suppressions live here:
`apply` (`:1007`, 150 lines) and a `Display::fmt` (`:2068`, 103 lines). The
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

**Acceptance gate:**
1. `reservation/mod.rs` contains only `mod` declarations, `use`/`pub use`, and
   module documentation.
2. No `too_many_lines` suppression remains under `crates/cargo-berth/src/reservation/`.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 1 added
`Reservation::is_active_for_coordination_run_and_worktree` and phase 3 made it
the single home for that predicate. It is an inherent method on `Reservation`,
so it moves with that type into the record cluster and needs no separate
re-export — re-exporting `Reservation` from the root keeps every caller's path
intact. Phase 3 also placed reservation-id ordering with `ReservationId` in
`ids.rs` precisely so this phase does not have to find a home for it here; do not
move it back. The `fmt` anchor moved to `:2068` after phase 1; confirm both
anchors in the current file before splitting.

---

### Phase 8 — `ledger/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `ledger/mod.rs` declares submodules and re-exports, and carries no logic.

**Spec:** Roughly 2,000 lines past its declarations, beside existing
`constants.rs`, `journal.rs`, `lock.rs`, and `projection.rs`. No
`too_many_lines` suppression lives here, so this phase is purely a move. The
clusters: the `Ledger` handle and its transaction driver; `WorktreeContext` and
its discovery; the identity files (`read_or_create_repo_instance_id`,
`create_or_read_worktree_id`, and the read-only variant); and the
coordination-run marker handling.

**Files:**
- `crates/cargo-berth/src/ledger/mod.rs`
- `crates/cargo-berth/src/ledger/handle.rs`
- `crates/cargo-berth/src/ledger/worktree_context.rs`
- `crates/cargo-berth/src/ledger/identity.rs`
- `crates/cargo-berth/src/ledger/session.rs`

**Acceptance gate:**
1. `ledger/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. The existing suite passes unmodified.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** the identity functions were renamed away from
the banned vocabulary before this plan started; keep the current names. Phase 1
reads the harness-session mapping through this module under the mutation lock —
`remove_current_session_mapping` acquires `MutationLock` before removing, and
that ordering is load-bearing, so it moves intact with the handle cluster. The
`dead_code` suppression in `ledger/journal.rs` belongs to phase 10, not here.

---

### Phase 9 — `board/mod.rs` and `gate/mod.rs` become tables of contents · status: todo

#### Work Order

**Goal:** Both roots declare submodules and re-export, and neither carries a
`too_many_lines` or `too_many_arguments` suppression.

**Spec:** `board/mod.rs` holds roughly 1,300 lines past its declarations beside
`tests.rs` and `tui.rs`, with three suppressions: `build` (`:566`, 179 lines),
`recorded_answers` (`:966`, 130 lines), and `append_authorization_answer`
(`:1101`, `too_many_arguments`). `gate/mod.rs` holds roughly 1,150 lines beside
`install.rs` and `permit.rs`, with no suppression — a pure move.

These two are paired in one phase because each is roughly half the size of the
earlier module phases and neither has a suppression cluster of its own.

`board/mod.rs` splits along row assembly, visibility and omission policy, and the
answer/disposition rendering. `append_authorization_answer` sits in the
answer-rendering cluster: its seven parameters are the audit row's complete
input, so give that cluster a parameter type that carries them rather than
suppressing the count. `gate/mod.rs` splits along reference-transaction
evaluation, branch rewrites and re-anchoring, and forced-permit auditing.
`board/tests.rs` is an existing sibling test module; move each test to sit with
the type it covers rather than leaving a catch-all.

**Files:**
- `crates/cargo-berth/src/board/mod.rs`
- `crates/cargo-berth/src/board/tests.rs`
- `crates/cargo-berth/src/board/rows.rs`
- `crates/cargo-berth/src/board/visibility.rs`
- `crates/cargo-berth/src/board/answers.rs`
- `crates/cargo-berth/src/gate/mod.rs`
- `crates/cargo-berth/src/gate/reference_transaction.rs`
- `crates/cargo-berth/src/gate/rewrite.rs`
- `crates/cargo-berth/src/gate/audit.rs`

**Acceptance gate:**
1. Both roots contain only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. No `too_many_lines` or `too_many_arguments` suppression remains under either
   directory.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 1 rendered the ambiguity outcome in
top-level `output.rs`, not in `board/mod.rs`, so no phase-1 rendering moves here.
Phase 3 placed reservation-id ordering with `ReservationId` in `ids.rs`;
`board/mod.rs` calls it and does not re-implement it.

---

### Phase 10 — Remove the remaining suppressions · status: todo

#### Work Order

**Goal:** No suppression remains anywhere in `crates/cargo-berth/`, except the
pre-authorized test-module boilerplate — `clippy::expect_used`, and
`clippy::panic` where the module uses `panic!`.

**Spec:** Four sites survive the earlier phases, in three shapes.

`crates/cargo-berth/tests/board.rs` holds two:
`release_dispositions_remain_resolved_when_trunk_rewrites` (`:820`, 122 lines)
and a `needless_pass_by_value` at `:4160`. The test splits into its arrangement
and its per-disposition assertions; the helper takes its payload by reference.

`crates/cargo-berth/src/cli.rs:559` suppresses `struct_excessive_bools` on the
resolve arguments, whose flags are one mutually exclusive disposition each.
Replace the flag set with semantic groups that convert immediately into
`ResolveDecision` at the Clap boundary, so the boolean count disappears rather
than being excused.

`crates/cargo-berth/src/ledger/journal.rs:334` suppresses `dead_code` on the
macro-generated `wire_name`, whose only consumer is the generated-contract drift
gate, and `crates/cargo-berth/src/ids.rs:132-138` carries a
`cfg_attr(not(test), expect(dead_code, …))` on the `uuid_identifier!` macro's
`future` constructor arm. Both are unused-outside-tests suppressions that author
a reason string, which this plan's binding constraint forbids. Give each
construct a real consumer, or delete the arm that has none.

Then sweep the whole crate and prove the claim: the only `#[allow]`/`#[expect]`
attributes left name `clippy::expect_used` or `clippy::panic` on a
`#[cfg(test)]` module, which
`~/rust/nate_style/rust/test-module-allow-boilerplate.md` pre-authorizes. A
`cfg_attr`-wrapped suppression counts; search for both spellings.

**Files:**
- `crates/cargo-berth/tests/board.rs`
- `crates/cargo-berth/src/cli.rs`
- `crates/cargo-berth/src/ledger/journal.rs`
- `crates/cargo-berth/src/ids.rs`

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

**Constraints from prior phases:** phases 4 through 9 removed every other
suppression; if one survives, it is that phase's defect, not a new item here.
The four sites named above were never owned by an earlier phase and are this
phase's own work.

## Gates

- Every phase: `verify.sh test cargo-berth` and `verify.sh lint cargo-berth`.
- Final: `verify.sh final`, plus `lint mend`, `lint clippy --workspace`, `lint doc`.
- No phase adds a suppression. No phase pushes.
