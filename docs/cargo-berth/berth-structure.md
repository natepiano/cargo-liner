# cargo-berth structure and selection fixes

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Removes every
> `clippy::too_many_lines` suppression by splitting the function it guards,
> turns the five module roots that carry logic back into tables of contents,
> and repairs the first-touch selection defect that lets replay order override
> an exact session reservation mapping.

> **As-built disposition: amend** — fold into
> `docs/cargo-berth/as-built/worktree-coordination.md`.

## Delegation Context

- **Project:** `cargo-berth` (workspace member of `cargo-liner`) — a git-worktree
  reservation engine coordinating path ownership and merge order between worktrees.
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

### Phase 1 — First-touch selection keeps the exact session reservation · status: todo

#### Work Order

**Goal:** A successful claim's session mapping survives the next pre-edit
first-touch check, so a later widen grows the reservation the session actually
holds rather than whichever one replay order reaches first.

**Spec:** `claim::acquire_first_touch` resolves an exact
`EditAuthorization::Session { coordination_run_id, reservation_id, worktree_id }`
but carries only the run and worktree into `reuse_first_touch_reservation`. That
function gathers every active reservation for the run and worktree, and
`partition_first_touch_protected_scopes` takes the first covering reservation in
replay order. Its `AlreadyHeld` outcome republishes that older reservation into
the session mapping, undoing the successful claim; the next PostToolUse drift
then widens the older reservation.

Introduce a semantic `FirstTouchReservationSelection` carried through locked
validation, with four states:

- `SessionMappedReservation` — an active mapping names a reservation; that exact
  reservation owns the already-held and widening outcomes, and replay order must
  not replace it.
- `SingleActiveRunReservation` — no usable mapping, exactly one eligible
  reservation for the run and worktree.
- `NoActiveRunReservation` — no usable mapping and nothing eligible.
- `AmbiguousActiveRunReservations` — no usable mapping and more than one eligible
  reservation. This state reaches the caller carrying the candidate ids; it
  appends nothing, widens nothing, and publishes no mapping.

The selection is resolved inside the ledger transaction that already holds the
lock, so the mapping cannot move between the read and the decision.

**Files:**
- `crates/cargo-berth/src/verb/claim.rs` — `acquire_first_touch`,
  `reuse_first_touch_reservation`, `partition_first_touch_protected_scopes`.
- `crates/cargo-berth/src/session/mod.rs` — `apply_journal_event` publishes every
  `Claim` and `Widen` identity; the mapping read that feeds the new selection.
- `crates/cargo-berth/src/reservation/mod.rs` — eligibility of an active
  reservation for a run and worktree.
- `crates/cargo-berth/src/output.rs` — rendering for the new ambiguity outcome.
- `crates/cargo-berth/tests/overlap.rs` — the two acceptance fixtures.

**Acceptance gate:**
1. A fixture claims reservation A, then claims overlapping reservation B in the
   same session, and proves the mapping names B immediately after the claim; the
   next first-touch check reports B and leaves the mapping on B; a later newly
   touched path widens B rather than A.
2. A fixture removes the usable session mapping while two active run reservations
   are eligible, and proves `AmbiguousActiveRunReservations` reaches the caller
   with the candidate ids and with no append, no widen, and no mapping
   publication.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** none — this is the first phase.

---

### Phase 2 — Split the generated-contract builders · status: todo

#### Work Order

**Goal:** `output_contract.rs` carries no `too_many_lines`, `dead_code`, or
`needless_pass_by_value` suppression, and the generated artifact is byte-identical
to the one checked in today.

**Spec:** Five functions and two other suppressions live in this file:

| Site | Function | Body |
| --- | --- | --- |
| `:276` | `outcome_rules` | 292 lines |
| `:773` | `generated_fixtures` | 264 lines |
| `:1065` | (`needless_pass_by_value`) | — |
| `:1288` | `render_python_tables` | 409 lines |
| `:1820` | `render_jq_validator` | 126 lines |
| `:2032` | `generated_python_exports_wire_name_discriminators` | 155 lines |
| `:2405` | (`dead_code` on a test-only type) | — |

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
- `crates/cargo-berth/src/output_contract.rs` — all seven sites.
- `docs/cargo-berth/generated/output-contract.json` — regenerate only if a doc
  comment changed; the split alone must not change it.

**Acceptance gate:**
1. No `#[allow]` or `#[expect]` remains in `output_contract.rs` other than the
   pre-authorized `clippy::expect_used` test-module boilerplate at `:1948`.
2. `generated_artifacts_are_reproducible_from_the_checked_in_contract` passes
   **without** the regenerate environment variable, proving the artifact did not
   move.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** none — this phase is independent of phase 1.

---

### Phase 3 — Split the reconciliation planners · status: todo

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
- `crates/cargo-berth/src/reconcile.rs` — both sites.

**Acceptance gate:**
1. No `too_many_lines` suppression remains in `reconcile.rs`.
2. The existing reconciliation tests pass unmodified — this phase changes no
   behavior.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** none.

---

### Phase 4 — `git/mod.rs` becomes a table of contents · status: todo

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

**Files:**
- `crates/cargo-berth/src/git/mod.rs` — reduced to declarations and re-exports.
- New submodules under `crates/cargo-berth/src/git/`.
- Call sites elsewhere only if a re-export cannot preserve the current path.

**Acceptance gate:**
1. `git/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. No `too_many_lines` suppression remains anywhere under `crates/cargo-berth/src/git/`.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 1 may have added a call into `git/` for
reachability during selection; re-export it under its existing path so that phase
needs no edit here.

---

### Phase 5 — `reservation/mod.rs` becomes a table of contents · status: todo

#### Work Order

**Goal:** `reservation/mod.rs` declares submodules and re-exports, and carries no
`too_many_lines` suppression.

**Spec:** Roughly 2,500 lines past its declarations, beside existing
`constants.rs`, `evidence.rs`, and `lifecycle.rs`. Two suppressions live here:
`apply` (`:1007`, 150 lines) and a `Display::fmt` (`:2057`, 103 lines). The
`fmt` is an exhaustive match over a large enum — split it by giving each variant
family its own renderer, keeping the outer match as the dispatch.

Split by type ownership: the retained-reservation set and its incursion
observation, the scope-partition logic, the reservation record and its replay
`apply`, and the conflict/holder evaluation are separate clusters that do not
appear in each other's field lists. Tests move with the type each one covers.

**Files:**
- `crates/cargo-berth/src/reservation/mod.rs` — reduced to declarations and re-exports.
- New submodules under `crates/cargo-berth/src/reservation/`.

**Acceptance gate:**
1. `reservation/mod.rs` contains only `mod` declarations, `use`/`pub use`, and
   module documentation.
2. No `too_many_lines` suppression remains under `crates/cargo-berth/src/reservation/`.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 1 touched reservation eligibility for a
run and worktree; that function moves with the cluster that owns it and stays
re-exported under its current path.

---

### Phase 6 — `ledger/mod.rs` becomes a table of contents · status: todo

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
- `crates/cargo-berth/src/ledger/mod.rs` — reduced to declarations and re-exports.
- New submodules under `crates/cargo-berth/src/ledger/`.

**Acceptance gate:**
1. `ledger/mod.rs` contains only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. The existing suite passes unmodified.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** the identity functions were renamed away from
the banned vocabulary before this plan started; keep the current names.

---

### Phase 7 — `board/mod.rs` and `gate/mod.rs` become tables of contents · status: todo

#### Work Order

**Goal:** Both roots declare submodules and re-export, and neither carries a
`too_many_lines` suppression.

**Spec:** `board/mod.rs` holds roughly 1,300 lines past its declarations beside
`tests.rs` and `tui.rs`, with two suppressions: `build` (`:566`, 179 lines) and
`recorded_answers` (`:966`, 130 lines). `gate/mod.rs` holds roughly 1,150 lines
beside `install.rs` and `permit.rs`, with no suppression — a pure move.

These two are paired in one phase because each is roughly half the size of the
earlier module phases and neither has a suppression cluster of its own.

`board/mod.rs` splits along row assembly, visibility and omission policy, and the
answer/disposition rendering. `gate/mod.rs` splits along reference-transaction
evaluation, branch rewrites and re-anchoring, and forced-permit auditing.
`board/tests.rs` is an existing sibling test module; move each test to sit with
the type it covers rather than leaving a catch-all.

**Files:**
- `crates/cargo-berth/src/board/mod.rs`, `crates/cargo-berth/src/board/tests.rs`.
- `crates/cargo-berth/src/gate/mod.rs`.
- New submodules under both directories.

**Acceptance gate:**
1. Both roots contain only `mod` declarations, `use`/`pub use`, and module
   documentation.
2. No `too_many_lines` suppression remains under either directory.
3. The existing suite passes unmodified.
4. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.

**Constraints from prior phases:** phase 1 may have added a board-visible
rendering for `AmbiguousActiveRunReservations`; it moves with the rendering
cluster.

---

### Phase 8 — Integration-test suppressions · status: todo

#### Work Order

**Goal:** No suppression remains anywhere in `crates/cargo-berth/`, except the
pre-authorized `clippy::expect_used` test-module boilerplate.

**Spec:** Two sites remain in `crates/cargo-berth/tests/board.rs`:
`release_dispositions_remain_resolved_when_trunk_rewrites` (`:820`, 122 lines)
and a `needless_pass_by_value` at `:4160`. The test splits into its arrangement
and its per-disposition assertions; the helper takes its payload by reference.

Then sweep the whole crate and prove the claim: the only `#[allow]`/`#[expect]`
attributes left name `clippy::expect_used` (or `clippy::panic` where the module
uses `panic!`) on a `#[cfg(test)]` module, which
`~/rust/nate_style/rust/test-module-allow-boilerplate.md` pre-authorizes.

**Files:**
- `crates/cargo-berth/tests/board.rs` — both sites.

**Acceptance gate:**
1. A crate-wide sweep shows no `too_many_lines`, `dead_code`, or
   `needless_pass_by_value` suppression.
2. Every surviving allow names only pre-authorized test lints, and each one's
   module actually uses the lint's pattern — no speculative allows.
3. `verify.sh test cargo-berth` and `verify.sh lint cargo-berth` both pass.
4. `bash ~/.claude/scripts/delegate/verify.sh final` passes, and
   `~/.claude/scripts/lint/lint mend`, `lint clippy --workspace`, and `lint doc`
   are all clean.

**Constraints from prior phases:** phases 2 through 7 removed every other
suppression; if one survives, it is that phase's defect, not a new item here.

## Gates

- Every phase: `verify.sh test cargo-berth` and `verify.sh lint cargo-berth`.
- Final: `verify.sh final`, plus `lint mend`, `lint clippy --workspace`, `lint doc`.
- No phase adds a suppression. No phase pushes.
