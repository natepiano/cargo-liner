# berth-flake

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** git 2.55 leaves a detached `git maintenance` child writing in a test repository after `git commit` returns. The template copy reads the repository while that child is still running. Every git a cargo-berth test starts, and every cargo-berth it starts, now runs with git's auto maintenance off.

> **Production: cargo-port-cleanup** — unit `berth-flake-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

The user, 2026-10-06 14:36 PDT: "you should start a new worktree to fix the cargo berth test - add a new unit using the process being worked on by the enh-showrunner session (you can ask if it's command is ready, otherwise do it by hand)" and "this unit shoudl fix the berth bug so it can be done in parallel".

## Evidence (showrunner, 2026-10-06)

- CI run 37533895283 on main at 4432028d, a commit that changed only a production doc, went red in Test Suite (Linux). The failing test was `cargo-berth::liveness active_checkpoint_release_reconciles_outstanding_retention_refs` (`crates/cargo-berth/tests/liveness.rs:1129`).
- It panicked at `crates/cargo-berth-test-support/src/directory_snapshot.rs:60:51` with `snapshot file should read: Os { code: 2, kind: NotFound }`. `DirectorySnapshot::capture` listed a file with `read_dir` that was gone by the time `fs::read` reached it.
- The capture runs in `RepositoryTemplate::instantiate` (`crates/cargo-berth-test-support/src/repository_template.rs:79`). It reads the shared per-test-binary template under `CARGO_TARGET_TMPDIR` with no lock, once the `.complete` marker exists (48889b3f). nextest runs each test in its own process, so many processes read the one template at the same time. Something removes a file inside it while another process reads it.
- The previous CI run on main (37531634387, 115a29ad) was green with the same cargo-berth code, so the failure is intermittent.

## Cause (berth-flake-unit, 2026-10-06)

**The file that disappears is `.git/objects/maintenance.lock`. It is created by the detached `git maintenance run --auto --detach` that the template build's last `git commit` starts.**

- `git commit` and the other porcelain commands start `git maintenance run --auto --quiet --detach` when they finish (GIT_TRACE2: `child_start[0] git maintenance run --auto --quiet --detach`). That process takes `.git/objects/maintenance.lock`, then forks a detached child.
- **git 2.54** (local): the forking parent's exit handler deletes the lock before `commit` returns (strace: the parent `unlink`s, then `exit_group`; the detached child's own `unlink` gets `ENOENT`). Nothing changes in the repository after `commit` returns.
- **git 2.55** (CI runner image ubuntu-24.04 20260927.320; checkout step: `git version 2.55.0`): the parent exits without deleting the lock. The detached child deletes it later: 0.44 ms after `commit` returned in one strace, and later than that on a loaded runner. In 50 back-to-back commits the lock was still present when `commit` returned 15 times on git 2.55 and 0 times on git 2.54.
- The liveness template build (`build_initialized_repository`, `liveness.rs:2183`) ends with a commit (`GIT.run_without_hooks(… "commit" …)`). `built_root` (`repository_template.rs:114`) then writes `.complete` (`:143`) at once, while the detached child still holds the lock. A test that starts in that window finds `.complete`. Its `capture` lists `.git/objects/maintenance.lock`, the child deletes it, and `fs::read` fails with `NotFound`. In CI the failing test was the first liveness test to finish (501/3754) and ran 0.079 s. It started just as another liveness test finished building the template.
- The dev shell does not supply git (`flake.nix` has no git package), so CI uses the runner's `/usr/bin/git`, and that git moves with the runner image.
- The test files already know this race. `ledger.rs:1556` and `drift.rs:2202`/`:4363` set `maintenance.auto false` and `gc.auto 0` in their own fixture repositories, after a geometric repack from these detached runs deleted a pack that a commit was still reading. The template builds and every other fixture lack that setting.
- Reproduced end to end with git 2.55 from nixpkgs-unstable first on `PATH`: 20 runs of `cargo nextest run -p cargo-berth --test liveness`, each after deleting the liveness template, panicked with `snapshot file should read: … NotFound` in 2 (runs 11 and 16). On git 2.54 the race does not occur.
- No liveness test writes the template. Each of the 30 tests was run alone with a template-change check after it, and none changed a file under the template after `.complete`.

## Delegation Context

- **Project:** the cargo-liner workspace. `cargo-berth` coordinates which worktree may change which paths. `cargo-berth-test-support` (`publish = false`) is the git invocation and repository fixtures its integration tests share.
- **Project started:** 2026-10-06T21:49:47Z
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-flake`, branch `fix/berth-flake`. Every command runs here.
- **Stack:** Rust 2024 edition; cargo nextest (through `verify.sh`); system git (2.54 on natedev, 2.55 on the CI runner).
- **Layout:**
  - `crates/cargo-berth-test-support/src/` — `git_driver.rs` (`git_command`, `GitDriver`), `berth_command.rs` (`berth_command`), `repository_template.rs` (`RepositoryTemplate`), `directory_snapshot.rs` (`DirectorySnapshot`), `lib.rs` (re-exports)
  - `crates/cargo-berth/tests/` — the integration test binaries; each defines its own `GIT: GitDriver`, its `git(…)` helper over `GIT.run`, and its `run_berth(…)` over `berth_command`
- **Key files:**
  - `crates/cargo-berth-test-support/src/git_driver.rs` — `EXECUTABLE_ENVIRONMENT` (`:17`), `HOOKS_DISABLED_CONFIGURATION` (`:20`), `git_command` (`:36`, the only `Command::new("git")` in the test tree: every `GitDriver` method goes through `prepare` (`:219`), which calls it), no test module yet
  - `crates/cargo-berth-test-support/src/berth_command.rs` — `CLAUDE_CODE_SESSION_ENVIRONMENT` (`:11`), `berth_command` (`:18`). Every test-started `cargo-berth` goes through it except `gate.rs:6606` (below)
  - `crates/cargo-berth-test-support/src/repository_template.rs` — module doc (`:1`–`13`), `RepositoryTemplate::build` field and doc (`:57`–`61`), `instantiate` (`:74`), `built_root` (`:114`), `.complete` written at `:143`, `remove_stale_builds` (`:166`)
  - `crates/cargo-berth-test-support/src/directory_snapshot.rs` — `capture_directory`, the panic at `:60`
  - `crates/cargo-berth/tests/ledger.rs:1556`–`1562` — per-repository `maintenance.auto false` / `gc.auto 0` and its comment
  - `crates/cargo-berth/tests/drift.rs:2202`–`2208` and `:4363`–`4369` — the same block twice
  - `crates/cargo-berth/src/git/command.rs:55`–`82` — cargo-berth's own git commands strip only `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`, `GIT_PREFIX` (`git/constants.rs:7`), and append to an inherited `GIT_CONFIG_COUNT` rather than replace it (`:71`), so `GIT_CONFIG_*` set on a test's `cargo-berth` reaches every git it starts
  - `crates/cargo-berth/tests/gate.rs:6606` — one trace test starts `cargo-berth` with `Command::new(executable)` and a wrapped `PATH`; it runs in its own copy, not a template
- **Test lanes:** `cargo-berth-test-support` — unit tests in `git_driver.rs`'s `#[cfg(test)] mod tests` (the crate has no `tests/`); `cargo-berth` — `crates/cargo-berth/tests`
- **Build:**
  - `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth-test-support`
  - `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:**
  - `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth-test-support`
  - `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:**
  - `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth-test-support`
  - `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  - Test-only change. No file under `crates/cargo-berth/src` changes, and `crates/cargo-berth/CHANGELOG.md` gets no entry.
  - Every process a cargo-berth test starts runs git with auto maintenance off. The tests start these processes through two helpers, `git_command` and `berth_command`, and the setting travels through the inherited environment to hooks and to cargo-berth's own git.
  - `RepositoryTemplate` keeps its unlocked readers: a template does not change after `.complete`, because no process the build started is left running in it.
  - Tests run through `verify.sh` (cargo nextest, user rule). Format with `cargo +nightly fmt` only (user rule). No test takes seconds: the new test runs one `git init` and one `git commit` (user rule).

## Calls made without the user

- **Fix in the shared helpers, not in each template build.** The template builds live in five test files (`board`, `edges`, `gate`, `lifecycle`, `liveness`), and the same detached runs already broke `ledger` and `drift`. One environment setting in `git_command` and `berth_command` covers all of them, and the three per-repository blocks are removed. Risk: a future test that starts git without these helpers runs maintenance again. The new test covers `git_command`, and the invariant in its doc names both helpers.
- **Environment, not repository config.** `GIT_CONFIG_COUNT` reaches git before `git init` writes any config, reaches hooks and cargo-berth's git without a `git config` call per repository, and leaves no trace in a template that `instantiate` would copy.
- **Not fixed here: a stale-build removal during a running test pass.** `remove_stale_builds` deletes another key's build. A relink of `target/debug/cargo-berth` in the same target directory partway through a run would change the key and delete a template that readers of the old key are still copying. CI never relinks during the test step, and single-user practice skips rare races (memory `single-user-skip-unlikely-edge-cases`). A shared lock for readers would close it if it is ever seen.
- **`gate.rs:6606` stays as it is.** Its `cargo-berth` runs in a private copy, so a maintenance child there cannot reach a template.

## Phases

### Phase 1 — Tests start git with auto maintenance off  · status: todo

#### Work Order

**Goal:** no git process that a cargo-berth test starts, directly, through a hook, or through `cargo-berth`, leaves a detached `git maintenance` running. A `RepositoryTemplate` is then unchanged from `.complete` on, so `instantiate` can no longer list a file that disappears.

**Spec:**

*The setting.* Set two git configuration entries through git's environment configuration on every command the two helpers build:

```
GIT_CONFIG_COUNT=2
GIT_CONFIG_KEY_0=maintenance.auto   GIT_CONFIG_VALUE_0=false
GIT_CONFIG_KEY_1=gc.auto            GIT_CONFIG_VALUE_1=0
```

- `maintenance.auto=false` stops `git commit`, `merge`, `rebase`, `am` and the other porcelain commands from starting `git maintenance run --auto --detach` at all (verified on git 2.55: with it set, a commit's GIT_TRACE2 holds no `maintenance`; without it, `child_start … git maintenance run --auto --quiet --detach`).
- `gc.auto=0` stops the commands that still call `git gc --auto` directly.
- Fixed slots 0 and 1 with a count of 2: the test owns the environment of the processes it starts, so an ambient `GIT_CONFIG_COUNT` in the developer's shell does not carry into them.

*Where.* One private function in `git_driver.rs` adds the three variables (count, keys, values) to a `Command`, from a const table of the two `(key, value)` pairs. `git_command` calls it. `berth_command` (`berth_command.rs:18`) calls it too, so a `cargo-berth` that runs git itself (gate merges and rebases) passes the setting on. Give the function the narrowest visibility that lets `berth_command.rs` call it, and let `cargo mend` settle it.

*Docs.*
- `git_command`'s doc gains one paragraph. Porcelain git starts a detached `git maintenance` that outlives the command: on git 2.55 its child still holds `.git/objects/maintenance.lock` after `git commit` returns, and deletes it later. A process left running in a fixture repository changes it under whoever reads it next: a `RepositoryTemplate` copy listing a file that then disappears, or a commit reading a pack a geometric repack deletes. So every git a test starts runs with auto maintenance off, and hooks and `cargo-berth` inherit the setting.
- `berth_command`'s doc gains one sentence saying it carries the same setting and why.
- `RepositoryTemplate::build`'s field doc (`repository_template.rs:57`–`61`) gains the requirement: the build must return with no process it started still running in the root, because every test copies the root without a lock once `.complete` exists. Build git through `git_command` or a `GitDriver`, and `cargo-berth` through `berth_command`.

*Remove the per-repository copies.* Delete the `maintenance.auto` / `gc.auto` `git config` calls and their comments at `ledger.rs:1556`–`1562`, `drift.rs:2202`–`2208` and `drift.rs:4363`–`4369`. Those repositories are driven only through `GIT` (`git_command`) and `run_berth` (`berth_command`), so the environment covers them. Their comment's geometric-repack account moves into `git_command`'s doc (above).

*The test.* In `git_driver.rs`, a `#[cfg(test)] mod tests` with one test, `commit_starts_no_background_maintenance`:
- in a `tempdir()`, run `git_command(<any executable string>)` with `init --quiet`, then `git_command(…)` with `-c user.name=… -c user.email=… commit --quiet --allow-empty -m …` and `GIT_TRACE2=<file in the tempdir>`
- assert both succeed and that the trace file holds no line containing both `child_start` and `maintenance`
- it fails without the fix on every git that starts detached maintenance (2.54 and 2.55 both log the `child_start`), and passes with it. It starts two git processes and runs in milliseconds.

**Files:**
- `crates/cargo-berth-test-support/src/git_driver.rs` — the setting, its use in `git_command`, the doc paragraph, the test module
- `crates/cargo-berth-test-support/src/berth_command.rs` — call the setting, one doc sentence
- `crates/cargo-berth-test-support/src/repository_template.rs` — the `build` field doc requirement
- `crates/cargo-berth/tests/ledger.rs` — remove the block at `:1556`–`1562`
- `crates/cargo-berth/tests/drift.rs` — remove the blocks at `:2202`–`2208` and `:4363`–`4369`

**Constraints from prior phases:** none.

**Acceptance gate:**
- `verify.sh test cargo-berth-test-support` passes and runs `commit_starts_no_background_maintenance`.
- `verify.sh test cargo-berth` passes.
- `verify.sh lint cargo-berth-test-support` and `verify.sh lint cargo-berth` pass.
- Smoke (unit director, outside `verify.sh`): git 2.55 from nixpkgs-unstable first on `PATH`, the liveness template deleted before each of 40 `cargo nextest run -p cargo-berth --test liveness` runs, 0 runs with `snapshot file should read`. Before the fix the same loop failed 2 of 20 runs, so 40 clean runs would occur by chance about 1.5% of the time if the race remained. A GIT_TRACE2 over one full `cargo-berth` test run holds no `maintenance` `child_start`.
