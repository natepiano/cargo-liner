# cargo-berth: test git and worktree discovery

## What it is

Every git a cargo-berth test starts runs with git's auto maintenance off. Porcelain git starts a detached `git maintenance run --auto` when it finishes, and on git 2.55 that child still writes in the repository after `git commit` returns: it raced `RepositoryTemplate` copies and could delete a pack another commit was reading. Separately, worktree discovery skips a `.git` that is not a git directory and honours `GIT_CEILING_DIRECTORIES`, so a test that runs `cargo-berth` outside any repository does not depend on what sits above TMPDIR, an empty `/tmp/.git` included.

## How it works

### Test git with auto maintenance off

All of it lives in `crates/cargo-berth-test-support` (`publish = false`, a dev-dependency of `cargo-berth`).

- `git_driver.rs`:
  - `AUTO_MAINTENANCE_DISABLED_CONFIGURATION` is the const table `[("maintenance.auto", "false"), ("gc.auto", "0")]`.
  - `pub(crate) fn disable_git_auto_maintenance(&mut Command)` sets it as `GIT_CONFIG_COUNT=2` with fixed slots `GIT_CONFIG_KEY_0`/`_1` and `GIT_CONFIG_VALUE_0`/`_1`. It replaces an ambient `GIT_CONFIG_COUNT` instead of appending to it, so the test process's own overlay does not carry into test processes.
  - `pub fn fixture_git_command() -> Command` is `Command::new("git")` with the setting applied, re-exported from `lib.rs`. It is the only `Command::new("git")` in the test tree. Every unit-test git helper in `crates/cargo-berth/src` builds on it: `git/fixture.rs` `run_git`, `board/test_support.rs` `git`, `gate/rewrite.rs` `git_output`, `worktree/liveness.rs` `run_git`, `ledger/test_support.rs` `scratch_repository`, and one-off commands in `reconcile.rs` and `git/reachability.rs`.
  - `pub fn git_command(executable)` builds on `fixture_git_command`, names the built binary in `CARGO_BERTH_EXECUTABLE` and clears `CLAUDE_CODE_SESSION_ID`. Every `GitDriver` method reaches it through `prepare`. Its doc records the detached-maintenance races.
- `berth_command.rs`: `berth_command(executable)` applies `disable_git_auto_maintenance` to the `cargo-berth` under test.
- `repository_template.rs`: the `RepositoryTemplate::build` field doc requires the build to return with no process it started still running in the root, and names `git_command`, `GitDriver` and `berth_command` as the way to start them.

The setting travels through the inherited environment. Git hands it to managed hooks. cargo-berth's own git (`git/command.rs`) strips only `GIT_DIR`, `GIT_WORK_TREE`, `GIT_INDEX_FILE`, `GIT_COMMON_DIR` and `GIT_PREFIX`, and its retention-ref hook suppression appends after an inherited `GIT_CONFIG_COUNT`, so the gate merges and rebases a test's `cargo-berth` runs also start no maintenance.

`commit_starts_no_background_maintenance` (the `git_driver.rs` test module) runs `git init` and an empty commit through `fixture_git_command` with `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`, traces the commit under `GIT_TRACE2`, and asserts that no `child_start` line names `maintenance`.

### Worktree discovery

`crates/cargo-berth/src/ledger/worktree_context.rs`:

- `is_git_directory(&Path)` holds for a directory with a `HEAD` file and `objects/` and `refs/` directories.
- `DiscoveryCeilings(Vec<PathBuf>)` (private) holds canonical ceiling directories.
  - `from_environment()` reads `GIT_CEILING_DIRECTORIES` with `env::var_os`; unset is an empty list.
  - `impl From<&OsStr>` parses a path list as git does, through `env::split_paths`.
  - `impl FromIterator<PathBuf>` keeps the absolute entries that canonicalize. Empty, relative and missing entries are dropped and the rest still apply; a non-UTF-8 entry is kept.
  - `contains(&Path)` compares canonical paths.
- `WorktreeContext::discover(dir)` calls `discover_with_ceilings(dir, &DiscoveryCeilings::from_environment())`.
- `discover_with_ceilings` canonicalizes the invocation directory and walks its `ancestors()`. Any directory other than the invocation directory that is a ceiling ends the walk before it is examined. At each examined directory, a `.git` passing `is_git_directory` is a main worktree, a `.git` file is followed through its `gitdir:` to a main or linked worktree, and anything else moves the walk up. An exhausted walk returns `LedgerError::RepositoryNotFound`.
- `from_registered_root`, for roots read from git's worktree registry, accepts any `.git` directory whose canonical path is the common git directory. It uses neither `is_git_directory` nor ceilings.

Module tests: `an_empty_git_directory_is_not_a_repository`, `discovery_stops_at_the_ceiling`, `the_invocation_directory_is_examined_even_at_the_ceiling`, `a_ceiling_at_the_invocation_directory_does_not_stop_the_walk`, `a_ceiling_list_entry_that_is_not_utf8_leaves_the_other_ceilings`. The bare-`.git` fixtures in `coordination_identity.rs` and `drift/execution.rs` create `HEAD`, `objects/` and `refs/`.

### Integration tests outside any repository

Four tests run `cargo-berth` in a temporary directory that must not resolve to a repository. Each sets `GIT_CEILING_DIRECTORIES` to that directory's parent on every `cargo-berth` it starts.

| Test | File | Where the ceiling is set |
| --- | --- | --- |
| `board_outside_a_git_worktree_reports_the_same_unreadable_facts_in_both_modes` | `tests/board.rs` | inline on its `berth_command` closure |
| `bare_repository_retains_repository_not_found_rejection` | `tests/ledger.rs` | inline, through the `GIT_CEILING_ENVIRONMENT` const |
| `an_unrecorded_binary_bypass_warns_without_blocking_the_ref_update` | `tests/gate.rs` | the `ceiling` parameter of `run_berth_with_input_and_environment` |
| `a_tool_call_outside_any_repository_states_nothing_to_the_hook` | `tests/hooks.rs` | `spawn_hook_verb_with_ceiling` for the hook, inline for the direct `drift --json` |

`spawn_hook_verb_with_ceiling` (`tests/support/reader_compat_hooks.rs`) sits beside the unchanged `spawn_hook_verb`; both call `spawn_hook_verb_for_discovery` with a private `RepositoryDiscovery::{Inherited, Ceiling(&Path)}`.

## Invariants

- Every git a cargo-berth test starts, unit tests included, directly, through a hook or through `cargo-berth`, runs with auto maintenance off. Test code starts git through `fixture_git_command`, `git_command`, `GitDriver` or `ledger/test_support::scratch_repository`, and `cargo-berth` through `berth_command`; never `Command::new("git")` or `Command::new(GIT_BINARY)`.
- Outside test code, only `git/command.rs` builds a git command.
- No fixture sets `maintenance.auto` or `gc.auto` per repository; the environment carries both.
- `RepositoryTemplate::build` returns with no process it started still running in the root. Template readers copy without a lock on that basis.
- Discovery always examines the invocation directory, a ceiling equal to it does not stop the walk, and the walk never moves up into a ceiling.
- Unit tests pass ceilings to `discover_with_ceilings` as values (`DiscoveryCeilings::from_iter`, `DiscoveryCeilings::from(&OsStr)`), never through the process environment. Integration tests set `GIT_CEILING_DIRECTORIES` only on the child command.
- `from_registered_root` and the `.git`-file branch of discovery keep their behavior.

## Calibration / gotchas

- **A hand-built git command brings detached maintenance back.** Under git 2.55 the child holds `.git/objects/maintenance.lock` after `git commit` returns and deletes it later, so a `RepositoryTemplate` copy lists a file that is gone by the time it is read (`snapshot file should read: … NotFound` from `DirectorySnapshot::capture`). In 50 back-to-back commits the lock was still present at return 15 times on git 2.55 and 0 times on 2.54. A commit can also read a pack that a geometric repack deletes.
- **git 2.54 hides the race.** Its forking parent deletes the lock before `commit` returns. The dev shell supplies no git, so CI runs the runner image's `/usr/bin/git`, which changes with the image. Reproduce with git 2.55 first on `PATH`.
- **The regression test ignores machine config.** `commit_starts_no_background_maintenance` sets `GIT_CONFIG_GLOBAL=/dev/null` and `GIT_CONFIG_NOSYSTEM=1`, so a developer's own `maintenance.auto=false` cannot hide a missing setting.
- **The gate trace test is the one exception.** It starts `cargo-berth` with `Command::new(executable)` and a wrapped `PATH` in `tests/gate.rs`, so its git runs maintenance. It runs in a private copy, not a template, so no reader can see that child.
- **A fixture `.git` needs three entries.** A test that makes a bare `.git` directory must also create `HEAD`, `objects/` and `refs/`, or discovery skips it.
- **Inlined environment policy.** `tests/board.rs`, `tests/hooks.rs` and `tests/ledger.rs` each inline a copy of their `run_berth` environment policy to add the ceiling. A change to that policy must reach the copies.
- **Canonical ceilings.** Ceilings and the invocation directory compare after canonicalization, so on macOS a `/tmp` ceiling matches a walk through `/private/tmp`.
- **Not handled:**
  - An ambient `GIT_CONFIG_PARAMETERS` (`-c maintenance.auto=…`) would take precedence over the overlay. No test path sets one.
  - `remove_stale_builds` deletes another key's template build. A relink of `target/debug/cargo-berth` partway through a run would change the key and delete a template that readers of the old key are still copying. CI never relinks during the test step; a shared reader lock would close it if it is ever seen.

## Why

- **Why the fix sits in the shared constructors:** template builds live in five test files (`board`, `edges`, `gate`, `lifecycle`, `liveness`), and the same detached runs had already broken fixtures in `ledger` and `drift`. One environment setting applied where commands are built covers all of them, so no fixture needs a config block of its own.
- **Why the environment and not repository config:** `GIT_CONFIG_COUNT` reaches git before `git init` writes any config, reaches hooks and cargo-berth's own git with no `git config` call per repository, and leaves nothing in a template that `instantiate` would copy. Per-repository config misses hooks, cargo-berth's git and any repository that does not set it.
- **Why unit tests are covered too:** with only the integration helpers fixed, a `GIT_TRACE2` over a full cargo-berth test run still held 323 detached maintenance starts, every one from a fixture helper in cargo-berth's own unit-test binary. One exported constructor in the existing dev-dependency covers them. A full run now traces 0 maintenance starts across 38,943 git processes.
- **Why template readers stay unlocked:** the race was a process left running after the build, not a reader-writer conflict. `instantiate` only copies a template once `.complete` exists (each liveness test, run alone, changed no file under it), so once the build leaves nothing running the template is immutable and readers need no lock.
- **Why discovery checks for a git directory:** an empty `.git` directory is not a repository to git, and accepting one let a test outside any repository resolve to a ledger under whatever sat above its temp directory. `HEAD`, `objects/` and `refs/` are the entries git itself requires.
- **Why ceilings follow git's rules:** a user or test that bounds git's discovery with `GIT_CEILING_DIRECTORIES` bounds cargo-berth's the same way, including git's treatment of the invocation directory and of empty, relative and missing entries.
- **Why tests pass ceilings as values:** `env::set_var` is `unsafe` in Rust 2024 and visible to every thread in the process. `discover_with_ceilings` takes the ceilings as an argument so the walk is tested without touching the environment.
- **Why `from_registered_root` accepts any `.git` directory:** its roots come from git's own worktree registry, so rejecting a partial `.git` there would contradict git.
