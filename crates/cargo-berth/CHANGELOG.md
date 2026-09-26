# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- A present non-trunk integration target's worktree holds a cover reservation against its own target. `target_uncovered` reports a present target with waiting reservations and no registered checkout.

- Report `target_missing` when an unlanded live reservation's recorded branch disappears; board JSON now includes each row's `target` and a sorted top-level `targets` list.
- Coordinate exclusive file and tree reservations across a Git repository's
  worktrees with an append-only journal and disposable projections.
- Record directed integration order, deferred overlaps, explicit override
  answers, checkpoints, release evidence, recovery decisions, and incursions.
- Guard trunk updates with an observe-or-enforce `reference-transaction` hook
  and report post-commit drift without rejecting an existing commit.
- Inspect current coordination state through a terminal board or its frozen
  `board --json` contract.
- Recover from projection loss, corrupt journals after confirmed review,
  orphaned worktrees, rewritten integration evidence, and deferred bypass
  audit records.

- Record each reservation's local integration target from `claim --target`, the claimant branch's `branch.<name>.cargoBerthTarget` setting, or the repository trunk; `init` pins older claims and `retarget` replaces a live target.

### Changed

- Ordering edges between reservations with the same target are judged at that target; edges across targets are judged at the repository trunk. The trunk gate holds an update that brings in a successor still held by a cross-target edge.

- Direct `release` leaves a reservation outstanding when its present target is uncovered; explicit `resolve --integrated-as` remains available. Containment judges each lane at its acting target, so a lane does not collide with that target's cover while work from `main` does.

- `release`, `resolve --integrated-as`, drift attribution, and rebase re-anchoring judge a reservation at its recorded target; malformed `--target` now names why it is not a local branch.

- Reconcile judges each reservation at its recorded target. A lane releases when it reaches its integration branch, and its merge extent holds only its own work.

- A linked worktree's `trunk` key no longer affects judgement; only the main worktree's key defines the repository trunk when its configuration exists.

### Fixed

- A lane merged into trunk ends its run even when its checkout holds an
  uncommitted edit to a tracked `.claude/config/berth.toml`. Uncommitted
  configuration changes no longer count as lane work in merge-extent or
  enrollment observations; committed changes still do.
- A lane whose history criss-crosses with trunk (each merged the other) gets a
  merge extent of the paths merging it would change or conflict on, instead of
  `merge_extent_unavailable` from `git diff --merge-base` failing with
  "multiple merge bases found". Branch paths now come from `git merge-tree
  --write-tree`, so a modify/delete conflict path is still protected.

- An orphaned or released reservation whose protected tip is not in trunk no
  longer suggests `cargo-berth resolve <id> --integrated-as <trunk tip>`. Both
  notices name that trunk as not containing the work; the orphan notice offers
  `--recovered`, `--retire-orphan`, or `--abandon`, and the lost-evidence notice
  asks for a trunk commit that carries the work. `resolve --integrated-as`
  refuses a trunk commit that carries neither the protected tip nor an
  equivalent of its scoped changes.

- A linked worktree added after `cargo-berth init` reads the main worktree's
  untracked `.claude/config/berth.toml` when it has none of its own, rather than
  reporting `unconfigured` and letting the edit hook allow every write in silence.
- The pre-edit hook honors `CARGO_BERTH_BYPASS=1`: it allows the edit before any
  engine invocation and leaves a pending-bypass marker, so a hung engine no longer
  blocks every write with no escape hatch.
- A post-commit drift check names only the paths the commit introduced or the
  working tree holds open, rather than re-reporting a path left unclaimed at every
  later commit on the branch.
- The recovery command an ambiguous first touch prints now carries
  `CARGO_BERTH_SESSION_ID=<session>`, so running it verbatim resolves the
  ambiguity instead of publishing no mapping and refusing the next edit.
- An incursion observation pairs each entered path with the holders that block it.
  Carried as two independent sets, an answered path stopped matching its own
  incident as soon as an unrelated path added a holder. The `incursion` record now
  writes `blocked_paths`; records in the two-array layout replay unchanged.
- The batched-attribution benchmark no longer sits in the test suite: it timed two
  hand-copied git command lines against each other with a 25ms margin and never
  ran cargo-berth at all.

### Notes

- The initial release coordinates one repository at a time. It does not select
  integration order, track project phases, or provide an editor write hook.
- The trunk gate ships in observe mode. Rejection is enabled per repository.
