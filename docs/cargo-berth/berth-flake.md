# berth-flake

> **Production: remove-running-section** — unit `berth-flake-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

The user, 2026-10-06 14:36 PDT: "you should start a new worktree to fix the cargo berth test - add a new unit using the process being worked on by the enh-showrunner session (you can ask if it's command is ready, otherwise do it by hand)" and "this unit shoudl fix the berth bug so it can be done in parallel".

## Evidence (showrunner, 2026-10-06)

- CI run 37533895283 on main at 4432028d, a commit that changed only a production doc, went red in Test Suite (Linux). The failing test was `cargo-berth::liveness active_checkpoint_release_reconciles_outstanding_retention_refs` (`crates/cargo-berth/tests/liveness.rs:1129`).
- It panicked at `crates/cargo-berth-test-support/src/directory_snapshot.rs:60:51` with `snapshot file should read: Os { code: 2, kind: NotFound }`. `DirectorySnapshot::capture` listed a file with `read_dir` that was gone by the time `fs::read` reached it.
- The capture runs in `RepositoryTemplate::instantiate` (`crates/cargo-berth-test-support/src/repository_template.rs:79`). It reads the shared per-test-binary template under `CARGO_TARGET_TMPDIR` with no lock, once the `.complete` marker exists (48889b3f). nextest runs each test in its own process, so many processes read the one template at the same time. Something removes a file inside it while another process reads it.
- The previous CI run on main (37531634387, 115a29ad) was green with the same cargo-berth code, so the failure is intermittent.

## Ask

Find what removes the file, and fix the cause so the copy can never see a half-changed template. Write the full phased plan here, send it to the showrunner (session cargo-liner), and wait for its approval before you run `/unit:delegate docs/cargo-berth/berth-flake.md`.
