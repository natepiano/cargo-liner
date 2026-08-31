# berth-structure — next items

Approved work this plan does not cover. Each item names a concrete target and
what would satisfy it. Adding an item commits nobody to building it; scheduling
one into a phase is the decision.

## 1. A contradictory proposal token re-proposes rather than claiming

**Target:** `crates/cargo-berth/tests/` (a lifecycle or answers case).

A proposal token is the serialized proposal, answer included. Spending a `defer`
token under `--before` re-gates at exit 3 with a refreshed proposal rather than
landing the claim — verified by hand against two worktrees, covered by no test.
The control spends cleanly at exit 0 `claimed`. The property is enforced where
the token lives, so the test belongs beside the engine, not in a front end.

Satisfied by: a test that answers `defer`, captures the token, re-invokes with
`--before` and that token, and asserts exit 3 plus a proposal whose answer is
`before`.

## 2. The re-proposal does not say why it re-gated

**Target:** `crates/cargo-berth/src/output.rs`, the proposal rendering.

When a contradictory token is spent, the engine correctly re-proposes, but the
rendered text is an ordinary proposal. It never says the supplied token
contradicted the answer it was presented with, so a reader sees the same screen
twice and cannot tell that anything was rejected.

Satisfied by: the re-gated proposal naming the contradiction — the answer the
token carried and the answer that was requested — as its own presentation block.

## 3. A worktree added after `cargo-berth init` reports `unconfigured`

**Target:** `crates/cargo-berth/src/config.rs` and the worktree discovery path.

`.claude/config/berth.toml` is untracked and per-worktree, so a `git worktree
add` performed after `init` produces a worktree the engine calls `unconfigured`.
Every verb then stops at a terminal outcome, which reads as a broken install
rather than as a new worktree.

Satisfied by: a new worktree of an initialized repository resolving the existing
configuration, or refusing with a diagnostic that names the real cause and the
command that fixes it.

## 4. `reference-transaction` bakes an absolute worktree path into a shared hook

**Target:** `crates/cargo-berth/src/gate/install.rs`.

`.git/hooks/reference-transaction` is shared by every worktree of a repository,
but the installed hook embeds the absolute path of the worktree that installed
it. A second worktree then runs the first one's hook, and removing that first
worktree leaves a hook pointing at a path that no longer exists.

Satisfied by: a hook that resolves its own repository and worktree at run time,
with a test that installs from one worktree and exercises the gate from another.

## 5. `CARGO_BERTH_BYPASS=1` still shells out to the engine

**Target:** `~/.claude/scripts/berth/install/hooks/berth_pre_edit.sh` (and, after
phase 6, the wrapper that replaces it).

The bypass is meant to be the escape hatch when the engine is the problem, but
the hook still invokes `cargo-berth` before honoring it. A binary that hangs or
crashes therefore cannot be bypassed, which is exactly when the bypass is needed.

Satisfied by: the bypass short-circuiting before any engine invocation, with the
pending-bypass marker still recorded for the audit.

## 6. `--integrated-as` requires a prior `release`, unreachable for an orphan

**Target:** `crates/cargo-berth/src/recovery.rs` (`recovery_operation`) and
`crates/cargo-berth/tests/liveness.rs`.

`cargo-berth resolve <id> --integrated-as <commit>` maps an active reservation to
`CheckpointRequired`. An orphaned reservation — one whose worktree is gone —
cannot run `release` from its holder worktree, so the disposition intended to
record rewritten integration cannot be reached.

Satisfied by: a liveness test creating an active orphan and a reachable trunk
commit, then either `--integrated-as` recording rewritten integration directly or
its refusal naming the command that can actually dispose of that orphan.

## 7. Neither `check` nor `claim` refuses a foreign same-worktree reservation

**Target:** `crates/cargo-berth/src/verb/claim.rs`.

A reservation held by a different coordination run in the *same* worktree does
not block a `check` or a `claim` from this run. Two sessions sharing a worktree
can therefore both believe they hold the same paths, which is the condition the
engine exists to prevent.

Satisfied by: same-worktree foreign reservations entering the refusal path with
their holder facts, and a test covering two runs in one worktree.

## 8. README engine-output quotes are checked against real engine renderings

**Target:** `crates/cargo-berth/README.md` and
`crates/cargo-berth/tests/engine_instructions.rs`.

The README presents engine output as verbatim `text` blocks, but no test ties
those blocks to the Rust renderings that produce them. Phase 3 changed drift and
resolve instructions and required four manual documentation corrections, and
every remaining phase that changes printed output can reintroduce the same drift.

Satisfied by: each README block presented as observed engine output being sourced
from a named real-binary scenario or frozen fixture, with a test that fails when
the quoted block differs from that rendering.

## 9. Drift attribution stays ambiguous between two reservations in one worktree

**Target:** `crates/cargo-berth/src/drift/` attribution and
`crates/cargo-berth/tests/liveness.rs`.

When one worktree holds two active reservations, drift cannot attribute a changed
path to either and refuses to widen, printing `DRIFT ATTRIBUTION REQUIRED … run
drift --reservation <id> with one listed reservation`. Because nothing resolves
the ambiguity, the notice fires again on the very next tool call, and it fired on
essentially every call across a full working session. The engine names the fix but
never applies it, so the paths stay unwidened indefinitely.

This is the drift-side companion to item 7: that item is about `check` and `claim`
not refusing a foreign same-worktree reservation, this one is about drift being
unable to choose between two of them.

Satisfied by: drift attributing a changed path to the reservation that already
covers it or that the session identity selects, refusing only when that genuinely
cannot be decided — and, when it does refuse, not repeating the same
unactionable notice on every subsequent invocation.

## 10. A post-commit drift check named an unmodified file as changed

**Target:** `crates/cargo-berth/src/drift/` change detection.

A post-commit drift check reported `Cargo.lock` as a changed path with ambiguous
attribution when that file was neither modified in the working tree nor part of
the commit. Observed once, at a checkpoint; not yet reduced to a repeatable case.

Satisfied by: a reproduction that pins what made an untouched path appear in the
changed set — a stale index read, a comparison against the wrong tree, or a
generation boundary — and a test covering it.
