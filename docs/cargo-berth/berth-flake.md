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
  - `crates/cargo-berth-test-support/src/git_driver.rs` — `EXECUTABLE_ENVIRONMENT` (`:17`), `HOOKS_DISABLED_CONFIGURATION` (`:20`), `fixture_git_command` (the one `Command::new("git")` in the test tree, with auto maintenance off), `git_command` (builds on it; every `GitDriver` method goes through `prepare`, which calls it), and a test module holding `commit_starts_no_background_maintenance`
  - `crates/cargo-berth-test-support/src/berth_command.rs` — `CLAUDE_CODE_SESSION_ENVIRONMENT` (`:11`), `berth_command` (`:18`). Every test-started `cargo-berth` goes through it except `gate.rs:6606` (below)
  - `crates/cargo-berth-test-support/src/repository_template.rs` — module doc (`:1`–`13`), `RepositoryTemplate::build` field and doc (`:57`–`61`), `instantiate` (`:74`), `built_root` (`:114`), `.complete` written at `:143`, `remove_stale_builds` (`:166`)
  - `crates/cargo-berth-test-support/src/directory_snapshot.rs` — `capture_directory`, the panic at `:60`
  - `crates/cargo-berth/tests/ledger.rs:1556`–`1562` — per-repository `maintenance.auto false` / `gc.auto 0` and its comment
  - `crates/cargo-berth/tests/drift.rs:2202`–`2208` and `:4363`–`4369` — the same block twice
  - `crates/cargo-berth/src/git/command.rs:55`–`82` — cargo-berth's own git commands strip only `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR`, `GIT_PREFIX` (`git/constants.rs:7`), and append to an inherited `GIT_CONFIG_COUNT` rather than replace it (`:71`), so `GIT_CONFIG_*` set on a test's `cargo-berth` reaches every git it starts
  - cargo-berth's unit tests start git through their own fixture helpers, each built on `cargo_berth_test_support::fixture_git_command()` since Phase 1: `git/fixture.rs` `run_git` (`:151`), `board/test_support.rs` `git` (`:443`), `gate/rewrite.rs` `git_output` (`:1449`), `worktree/liveness.rs` `run_git` (`:547`), `ledger/test_support.rs` `scratch_repository` (`:9`, which sets `maintenance.auto` / `gc.auto` per repository at `:17`–`31`), and two one-off commands at `reconcile.rs:5535` and `git/reachability.rs:1508`
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
  - Test-only change. Under `crates/cargo-berth/src` only `#[cfg(test)]` code changes, and `crates/cargo-berth/CHANGELOG.md` gets no entry.
  - Every process a cargo-berth test starts runs git with auto maintenance off. The tests start these processes through two helpers, `git_command` and `berth_command`, and the setting travels through the inherited environment to hooks and to cargo-berth's own git.
  - `RepositoryTemplate` keeps its unlocked readers: a template does not change after `.complete`, because no process the build started is left running in it.
  - Tests run through `verify.sh` (cargo nextest, user rule). Format with `cargo +nightly fmt` only (user rule). No test takes seconds: the new test runs one `git init` and one `git commit` (user rule).

## Calls made without the user

- **Fix in the shared helpers, not in each template build.** The template builds live in five test files (`board`, `edges`, `gate`, `lifecycle`, `liveness`), and the same detached runs already broke `ledger` and `drift`. One environment setting in `git_command` and `berth_command` covers all of them, and the three per-repository blocks are removed. Risk: a future test that starts git without these helpers runs maintenance again. The new test covers `git_command`, and the invariant in its doc names both helpers.
- **Environment, not repository config.** `GIT_CONFIG_COUNT` reaches git before `git init` writes any config, reaches hooks and cargo-berth's git without a `git config` call per repository, and leaves no trace in a template that `instantiate` would copy.
- **Not fixed here: a stale-build removal during a running test pass.** `remove_stale_builds` deletes another key's build. A relink of `target/debug/cargo-berth` in the same target directory partway through a run would change the key and delete a template that readers of the old key are still copying. CI never relinks during the test step, and single-user practice skips rare races (memory `single-user-skip-unlikely-edge-cases`). A shared lock for readers would close it if it is ever seen.
- **cargo-berth's unit tests are in this phase.** After the first fix, a GIT_TRACE2 over a full cargo-berth test run still held 323 detached maintenance starts, every one from a fixture helper inside cargo-berth's own unit-test binary and none from the integration tests. The Goal covers every git a cargo-berth test starts, so this phase takes them, through one exported constructor in `cargo-berth-test-support`, already a dev-dependency of `cargo-berth`. Risk: a later unit-test helper that builds git by hand starts maintenance again; the constructor's doc names the rule.
- **`gate.rs:6606` stays as it is.** Its `cargo-berth` runs in a private copy, so a maintenance child there cannot reach a template.

## Phases

### Phase 1 — Tests start git with auto maintenance off  · status: done

#### As-built

- Every git a cargo-berth test starts, directly, through a hook, or through `cargo-berth`, runs with `maintenance.auto=false` and `gc.auto=0`. `disable_git_auto_maintenance` in `git_driver.rs` sets them from a const table as `GIT_CONFIG_COUNT=2` with fixed slots `GIT_CONFIG_KEY_0`/`_1` and `GIT_CONFIG_VALUE_0`/`_1`, so an ambient `GIT_CONFIG_COUNT` does not carry into test processes.
- `pub fn fixture_git_command() -> Command` (re-exported from `lib.rs`) is `Command::new("git")` with the setting applied. `git_command(executable)` builds on it, and `berth_command` applies the same setting, so managed hooks and cargo-berth's own git (gate merges and rebases) inherit it.
- cargo-berth's unit-test helpers under `crates/cargo-berth/src` start git only through `cargo_berth_test_support::fixture_git_command()`. No per-repository `maintenance.auto` / `gc.auto` `git config` remains in `tests/ledger.rs`, `tests/drift.rs` or `scratch_repository`.
- `RepositoryTemplate::build` must return with no process it started still running in the root, because every test copies the root without a lock once `.complete` exists.
- `commit_starts_no_background_maintenance` (`git_driver.rs`) runs `init` and an empty commit through `fixture_git_command` under `GIT_TRACE2`, with global and system config ignored, and asserts no `child_start` line names `maintenance`. A full cargo-berth run traces 0 maintenance starts across 38,943 git processes; 40 liveness runs under git 2.55 show no NotFound.

**Files:**
- `crates/cargo-berth-test-support/src/git_driver.rs` — `disable_git_auto_maintenance`, `fixture_git_command`, `git_command` doc on the detached-maintenance races, the regression test
- `crates/cargo-berth-test-support/src/berth_command.rs` — applies the setting
- `crates/cargo-berth-test-support/src/repository_template.rs` — the `build` no-running-process requirement
- `crates/cargo-berth-test-support/src/lib.rs` — re-exports `fixture_git_command`
- `crates/cargo-berth/src/{git/fixture.rs, board/test_support.rs, gate/rewrite.rs, worktree/liveness.rs, ledger/test_support.rs, reconcile.rs, git/reachability.rs}` — unit-test git through `fixture_git_command`
- `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/drift.rs` — no per-repository maintenance config

**Binds later work:** any git a new cargo-berth test starts, unit tests included, comes from `cargo_berth_test_support::fixture_git_command()` (or `git_command` / `GitDriver` / `berth_command` / `ledger/test_support::scratch_repository()`, which carry the same setting), never `Command::new("git")` or `Command::new(GIT_BINARY)`. Outside test code, only `git/command.rs` builds git directly.

**Gotchas:** a hand-built git command brings detached maintenance back. Under git 2.55 its child holds `.git/objects/maintenance.lock` after `git commit` returns and deletes it later, so a `RepositoryTemplate` copy lists a file that then disappears (NotFound), and a commit can read a pack that a geometric repack deletes.

**Ruled out:** ambient `GIT_CONFIG_PARAMETERS` overriding the setting (no test path sets `maintenance.auto` through `-c`); per-repository `git config` instead of the environment (misses hooks, `cargo-berth`'s own git and any repository that does not set it).

### Phase 2 — Discovery ignores an empty `.git` and stops at git's ceiling  · status: todo

**Follow-up 1 of 1** (showrunner, 2026-10-06) · then: plan done.

#### Work Order

**Goal:** a `.git` directory that is not a git directory no longer makes cargo-berth treat its parent as a worktree, and the four "outside any repository" tests stop depending on what sits above their temporary directory. On natedev an empty `/tmp/.git` appears while a Codex seat's sandbox runs. In this phase's review it made four tests fail every time: `board_outside_a_git_worktree_reports_the_same_unreadable_facts_in_both_modes` (`board.rs:1152`), `bare_repository_retains_repository_not_found_rejection` (`ledger.rs:1439`), `an_unrecorded_binary_bypass_warns_without_blocking_the_ref_update` (`gate.rs:3824`) and `a_tool_call_outside_any_repository_states_nothing_to_the_hook` (`hooks.rs:1113`). Each got `unconfigured` ("no cargo-berth configuration at /tmp/.claude/config/berth.toml") where it expected `ledger_unreadable` or exit 4.

**Spec:**

*A git directory's structure.* `WorktreeContext::discover` (`crates/cargo-berth/src/ledger/worktree_context.rs:72`–`105`) walks `invocation_directory.ancestors()` and accepts any `dot_git.is_dir()` (`:76`). It calls one private function that accepts a `.git` directory only when it holds the structure git requires of a git directory: a `HEAD` file, an `objects` directory and a `refs` directory. A `.git` that fails the check is skipped and the walk goes on to the next ancestor, as git does. The check is structural only; it does not validate `HEAD`'s contents as git does, which an empty or partial `.git` never reaches. `from_registered_root` (`:116`–`117`) is unchanged: git itself reported those roots, and rejecting one there would turn a registered checkout into no checkout for liveness and reconciliation. The `.git` file branch (`read_git_directory_file`, `:390`) is unchanged.

*Git's ceiling.* `discover` honours `GIT_CEILING_DIRECTORIES` as git's own discovery does: a colon-separated list of absolute paths, empty entries ignored (git uses one only to skip symlink resolution, and this walk compares canonical forms throughout). The invocation directory is always examined, even when it is itself a ceiling; the walk never moves up into a ceiling directory. Compare paths in canonical form, so a test can name its temporary directory's parent. The ceiling list reaches the private discovery function as one named type (for example `DiscoveryCeilings`), parsed once from the environment by `discover`, never as a bare slice or `Option`. `git::repository_root` (`git/discovery.rs:48`) already gets this from git, because `git/command.rs` passes the variable through.

*Fixtures that relied on an empty `.git`.* The unit tests at `crates/cargo-berth/src/coordination_identity.rs:1001`–`1003` and `crates/cargo-berth/src/drift/execution.rs:919`–`922` build a bare empty `.git` and expect `discover` to accept it. Give them a `.git` that passes the new check (`HEAD`, `objects/`, `refs/`), or `test_support::scratch_repository()` (`ledger/test_support.rs:7`).

*Regression tests,* in `worktree_context.rs`'s `mod tests` (`:436`, modelled on `git_file_without_common_directory_is_a_main_worktree`, `:450`):
- `an_empty_git_directory_is_not_a_repository`: an empty `.git` under the tempdir, then `discover` from a child directory reports `RepositoryNotFound` when the tempdir's parent is the ceiling.
- `discovery_stops_at_the_ceiling`: a real repository whose child directory is named as the ceiling, then `discover` from below it reports `RepositoryNotFound`.
- `the_invocation_directory_is_examined_even_at_the_ceiling`: a real repository's root named as the ceiling, then `discover` from that root finds the repository, as git does.

Each test passes its ceiling explicitly (a parameter or an inner function that takes the ceiling list), never by setting the process environment: `cargo test` runs many tests in one process, where environment writes race.

*The four tests.* Each sets `GIT_CEILING_DIRECTORIES` to its temporary directory's parent on every `cargo-berth` it starts, including the hook process in `hooks.rs` (`run_post_tool_use` → `spawn_hook_verb`, `tests/support/reader_compat_hooks.rs:54`). `spawn_hook_verb` keeps its signature, because `reader_compat.rs:278` also calls it; the hook test passes the ceiling through a variant beside it that takes the extra environment. Then a repository at `/tmp`, or above `/tmp/claude`, cannot reach them.

*CHANGELOG.* One entry in `crates/cargo-berth/CHANGELOG.md` under the unreleased heading: an empty or partial `.git` directory is no longer taken for a repository, and discovery honours `GIT_CEILING_DIRECTORIES`.

**Files:**
- `crates/cargo-berth/src/ledger/worktree_context.rs` — the git-directory check, the ceiling, three regression tests
- `crates/cargo-berth/src/coordination_identity.rs`, `crates/cargo-berth/src/drift/execution.rs` — their fixtures' `.git`
- `crates/cargo-berth/CHANGELOG.md` — the entry
- `crates/cargo-berth/tests/board.rs`, `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/gate.rs`, `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/tests/support/reader_compat_hooks.rs` — the ceiling on the four tests

**Seats:** 1 writer + 1 tester
- `impl`: `worktree_context.rs`, `coordination_identity.rs`, `drift/execution.rs`, `CHANGELOG.md`
- `test`: `tests/board.rs`, `tests/ledger.rs`, `tests/gate.rs`, `tests/hooks.rs`, `tests/support/reader_compat_hooks.rs`

**Constraints from prior phases:** Phase 1 turned off git's auto maintenance in `git_command` and `berth_command`, and in `fixture_git_command` (`cargo-berth-test-support`, re-exported from its `lib.rs`), which cargo-berth's own unit tests use. Leave that intact. Any git a new unit test under `crates/cargo-berth/src` starts comes from `cargo_berth_test_support::fixture_git_command()`, or from `ledger/test_support::scratch_repository()`, which uses it; never `Command::new("git")`. Phase 1's "test-only change" invariant does not hold here: this phase changes `crates/cargo-berth/src` and takes a CHANGELOG entry.

**Acceptance gate:**
- `verify.sh test cargo-berth` passes, with the three new unit tests included.
- `verify.sh lint cargo-berth` passes.
- Smoke (unit director): with an empty `/tmp/.git` created by hand, the four tests pass (`verify.sh test cargo-berth --no-cache --filter …` for each), and `/tmp/.git` is removed afterwards.
