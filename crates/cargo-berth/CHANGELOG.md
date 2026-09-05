# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

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

### Fixed

- A linked worktree added after `cargo-berth init` reads the main worktree's
  `.claude/config/berth.toml` when it has none of its own. The file is untracked,
  so `git worktree add` never carried it, and the new worktree reported
  `unconfigured`: every verb stopped there, and the edit hook allowed every write
  in silence. A file the linked worktree does have still wins, and a worktree with
  no file anywhere names the main worktree's path as the one `init` should create.
- The pre-edit hook honors `CARGO_BERTH_BYPASS=1`. The wrapper only checked that
  the engine was on `PATH` and then replaced itself with it, so a hung or crashed
  engine blocked every write with no escape hatch. The wrapper now allows the
  edit before any engine invocation and leaves a pending-bypass marker whose
  `action` is `editing`; the engine honors the variable the same way when
  invoked directly, and marker recovery records the action a marker names.
- An incursion observation pairs each entered path with the holders that block
  it. The observation and the retained incident carried paths and holders as two
  independent sets, so a caller could report every path under the union of all
  their holders; an answered path then stopped matching its own incident as soon
  as an unrelated path added a holder, and was raised again. The `incursion`
  journal record now writes `blocked_paths`; records already written in the
  two-array layout replay unchanged.
- The batched-attribution benchmark no longer sits in the test suite. It timed
  two hand-copied git command lines against each other with a 25ms margin, so
  it went red whenever the machine was busy and never ran cargo-berth at all.
  The property it meant to pin, one `git log` for any number of paths and
  commits, is already asserted by the post-commit cardinality matrix through
  the real engine.

### Notes

- The initial release coordinates one repository at a time. It does not select
  integration order, track project phases, or provide an editor write hook.
- The trunk gate ships in observe mode. Rejection is enabled per repository.
