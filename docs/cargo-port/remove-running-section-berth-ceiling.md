# berth-ceiling

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-berth reads `GIT_CEILING_DIRECTORIES` entry by entry as git does, and the `ledger::worktree_context` ceiling tests pass on the Mac and on natedev. That clears the way to offload cargo-berth tests to the Mac.

> **Production: remove-running-section** — unit `berth-ceiling-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-14 14:44 EDT (the showrunner's brief; "as git does" and "kept as given" are the showrunner's words, and the diagnosis in it was the showrunner's, corrected below)

cargo-berth: GIT_CEILING_DIRECTORIES entries are not canonicalized, so on macOS a ceiling under /var never matches the canonical /private/var walk (src/ledger/worktree_context.rs discover_with_ceilings canonicalizes only the invocation directory). Canonicalize each ceiling as git does, keep entries that cannot be canonicalized as given, and make the ledger::worktree_context ceiling tests pass on the Mac (ssh mac) as well as natedev. Small fix: one phase, fast; then cargo-berth tests can be offloaded to the Mac.

## Findings before planning

Measured 2026-10-14 on the Mac's `main` checkout at 50fdf062 (the code this branch starts from), through `ssh mac` with `zsh -lc`, so TMPDIR is `/var/folders/…/T`:

- `cargo nextest run -p cargo-berth --no-fail-fast`: 1000 tests, 998 passed, 2 failed, in 165.6 s. One of the passes was flagged leaky: `cargo-berth::answers a_defer_answer_adds_no_blocking_holder`.
- The two failures are `ledger::worktree_context::tests::the_invocation_directory_is_examined_even_at_the_ceiling` (line 549) and `…::a_ceiling_at_the_invocation_directory_does_not_stop_the_walk` (line 562). Each asserts `context.repository_root() == repository.path()`. The left side is canonical (`/private/var/folders/…/T/.tmpXXXX`) and the right side is the tempdir as given (`/var/folders/…/T/.tmpXXXX`). The defect is in the tests' expected value, not in the walk.
- Ceilings are already canonicalized: `impl FromIterator<PathBuf> for DiscoveryCeilings` (line 54) runs `fs::canonicalize` on every absolute entry, and `discovery_stops_at_the_ceiling` passes on the Mac. Two things still differ from the Source:
  - An entry that fails to canonicalize is dropped (`filter_map(… .ok())`). The brief asks for it to be kept as given.
  - git's rule for an empty entry is missing. git's `canonicalize_ceiling_entry` (`setup.c`) skips an empty entry, then keeps every later absolute entry *as given, without canonicalizing it*, so that later ceilings are never resolved. cargo-berth drops empty entries and canonicalizes everything after them.

## Delegation Context

- **Project:** `cargo-berth`, the worktree reservation ledger and hook engine. Discovery finds the containing worktree from `.git` metadata alone, without running git.
- **Project started:** 2026-10-14T18:53:51.781+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-berth-ceiling`, branch `remove-running-section-berth-ceiling`. Every seat works only here, and every Work Order names it. The showrunner set both (production doc, Units table).
- **Base commit:** `edc0f0ebf711f1b63b0e1ef68a92312f2776e09a`. The Mac clone `/Users/natemccoy/rust/cargo-liner-berth-ceiling` is checked out detached there.
- **Stack:** Rust workspace and cargo-nextest. The crate has no lib target: unit tests live in the binary `cargo-berth::bin/cargo-berth`, so select them with `--bins`, not `--lib`.
- **Key files:**
  - `crates/cargo-berth/src/ledger/worktree_context.rs`:
    - `DiscoveryCeilings` (31), `from_environment` (35), `contains` (43).
    - `impl From<&OsStr>` (48) parses through `env::split_paths`. An empty list entry comes through as an empty `PathBuf`.
    - `impl FromIterator<PathBuf>` (54).
    - `WorktreeContext::discover` (116), `discover_with_ceilings` (121), `repository_root()` (291).
    - Tests from 495: `an_empty_git_directory_is_not_a_repository` (514), `discovery_stops_at_the_ceiling` (528), `the_invocation_directory_is_examined_even_at_the_ceiling` (542), `a_ceiling_at_the_invocation_directory_does_not_stop_the_walk` (553), `a_ceiling_list_entry_that_is_not_utf8_leaves_the_other_ceilings` (566).
  - `crates/cargo-berth/src/ledger/test_support.rs` — `scratch_repository()` returns a `TempDir` with `git init` run in it. Its path is the tempdir as given, which is not canonical.
  - `docs/cargo-berth/as-built/test-git-and-discovery.md` — lines 29–39 describe `DiscoveryCeilings` and list the module tests.
  - `crates/cargo-berth/CHANGELOG.md` — `## [Unreleased]` has `### Added` and `### Changed`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-berth`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - Discovery always examines the invocation directory. A ceiling equal to the invocation directory does not stop the walk, and the walk never moves up into a ceiling (as-built `test-git-and-discovery.md`).
  - The walk compares canonical ancestors of the canonical invocation directory against the stored ceilings. That part does not change.
  - Every git a test starts goes through `fixture_git_command` or `scratch_repository`, never `Command::new("git")` (as-built invariant).
  - No test takes a second (user rule, memory `no-multi-second-tests`).
  - Format with `cargo +nightly fmt` only (memory `rustfmt-nightly-only`).
  - `verify.sh` waits by itself for a natedev build hold. Run one `verify.sh` at a time (memory `verify-runs-one-at-a-time`).
  - The Mac's `/Users/natemccoy/rust/cargo-liner` is the user's checkout on `main`. Leave it alone.

## Phases

### Phase 1 — Ceilings read as git reads them, and the ceiling tests pass on the Mac  · status: done

**Depends on:** none

#### Work Order

**Worktree:** `/home/natepiano/rust/cargo-liner-berth-ceiling`, branch `remove-running-section-berth-ceiling`.

**Goal:** `DiscoveryCeilings` treats each `GIT_CEILING_DIRECTORIES` entry as git's `canonicalize_ceiling_entry` does, except that an entry which cannot be canonicalized is kept as given. Every `ledger::worktree_context` test passes on natedev and on the Mac. The full `cargo-berth` suite passes on the Mac, the route to offloading it there.

**Spec:**

1. **`impl FromIterator<PathBuf> for DiscoveryCeilings`** walks the entries in order. Replace the two-filter chain with one fold or loop that follows these rules:
   - A relative entry is dropped, wherever it sits.
   - An empty entry is dropped. From then on, every later absolute entry is kept as given, without `fs::canonicalize`. This is git's rule.
   - An absolute entry before any empty entry is replaced by its `fs::canonicalize` result. When canonicalization fails, the entry is kept as given (showrunner's brief). It cannot match a canonical ancestor of an existing directory, so it bounds nothing, and it never blocks the other entries.
   - The non-UTF-8 behaviour stays as it is, and its test keeps passing.
2. **Doc comments** say what each item does now:
   - On the struct: "Ceiling directories that bound filesystem worktree discovery, canonicalized as git canonicalizes them."
   - On `From<&OsStr>`: it parses the path list as git does.
   - On `FromIterator`: the three rules above, with a pointer to git's `setup.c` `canonicalize_ceiling_entry`.
3. **Fix the two Mac failures.** In `the_invocation_directory_is_examined_even_at_the_ceiling` and `a_ceiling_at_the_invocation_directory_does_not_stop_the_walk`, compare `context.repository_root()` with `fs::canonicalize(repository.path())`, not with the path as given. Change nothing else in those tests.
4. **New module tests.** Each one is pure, runs well under a second, and fails on Linux when its rule is broken:
   - `a_ceiling_named_through_a_symlink_stops_the_walk` — the Mac's `/var` case, rebuilt on Linux. Take a scratch repository with `ceiling/child`. Make a second `tempdir()` that holds a symlink `link → <repo>/ceiling`, created with `std::os::unix::fs::symlink`. Build `DiscoveryCeilings::from_iter([link])`. Discovery from `child` returns `RepositoryNotFound`.
   - `a_ceiling_after_an_empty_entry_is_kept_as_given` — the same layout, with the list `":" + link` parsed through `DiscoveryCeilings::from(&OsStr)`. Discovery from `child` finds the repository, so the symlinked ceiling no longer stops the walk, which is git's behaviour. Also assert that the stored entries equal `[link]`.
   - `a_ceiling_that_cannot_canonicalize_is_kept_as_given` — `from_iter` of an absolute path under a tempdir that does not exist. The stored entries equal that path. The test module reads the private tuple field directly.
   - `a_relative_ceiling_is_dropped` — `from_iter([PathBuf::from("relative")])` stores nothing.
5. **As-built.** `docs/cargo-berth/as-built/test-git-and-discovery.md`:
   - Rewrite the `FromIterator` bullet (line 32) to state the three rules.
   - Change `DiscoveryCeilings(Vec<PathBuf>) … holds canonical ceiling directories` to "holds ceiling directories, canonical except entries after an empty entry and entries that fail to canonicalize".
   - Add the four new test names to the module-test list (line 39).
6. **The leaky test (showrunner add-on, 2026-10-14).** `cargo-berth::answers a_defer_answer_adds_no_blocking_holder` (`crates/cargo-berth/tests/answers.rs:1701`) passed on the Mac but was flagged `LEAK`: some process it started still held its stdout or stderr when the test ended. A gate run on the Mac should come back clean.
   - Find what leaks. Run that test repeatedly on the Mac clone, then run the `answers` binary repeatedly, and name the process that survives the test.
   - If the cause is in the test or its helpers, fix it in this phase: the test waits for the child or no longer hands it the pipe. Add the file to the Files below.
   - If the cause is in product code, do not fix it here. Record the measured cause and send it to the showrunner as a proposal.
7. **Changelog.** `crates/cargo-berth/CHANGELOG.md`, `## [Unreleased]` → `### Changed`, one entry: "`GIT_CEILING_DIRECTORIES` is read as git reads it: entries after an empty entry are used as given, without resolving symlinks, and an absolute entry that cannot be resolved is kept as given instead of being dropped."

**Files:**
- `crates/cargo-berth/src/ledger/worktree_context.rs` — `DiscoveryCeilings` impls, doc comments, module tests
- `docs/cargo-berth/as-built/test-git-and-discovery.md` — discovery section
- `crates/cargo-berth/CHANGELOG.md` — one Changed entry

**Seats:** 1 writer + 1 tester. The Work Order's ceiling work all lands in one file, so the tester takes the leak instead.
- `impl` — Spec items 1–5 and 7: `crates/cargo-berth/src/ledger/worktree_context.rs` (its in-module tests included), the as-built doc and the changelog.
- `test` — Spec item 6: reproduces the leak in the Mac clone (`/Users/natemccoy/rust/cargo-liner-berth-ceiling`, at the base commit, made by the unit director) and owns `crates/cargo-berth/tests/answers.rs` and the `crates/cargo-berth/tests/support/` helpers that test uses, if the cause is there.

**Mac lane:**
1. Clone once on the Mac, from its local repository so no network is needed: `ssh mac 'git clone --quiet /Users/natemccoy/rust/cargo-liner /Users/natemccoy/rust/cargo-liner-berth-ceiling'`. Skip this step if the clone exists. Its tree sits outside `/tmp` on purpose.
2. Bring the base commit over:
   - On natedev: `git -C /home/natepiano/rust/cargo-liner-berth-ceiling bundle create <session dir>/base.bundle 50fdf062..remove-running-section-berth-ceiling`.
   - `scp` the bundle to the Mac.
   - On the Mac: `git -C /Users/natemccoy/rust/cargo-liner-berth-ceiling fetch <bundle> HEAD`, then `checkout --detach edc0f0ebf711f1b63b0e1ef68a92312f2776e09a`.
   - Check that `git status --short` is empty.
3. `scp` every changed candidate source file into the clone: `crates/cargo-berth/src/ledger/worktree_context.rs`, plus any test file item 6 changes. Each `sha256sum` must match natedev's.
4. Run the suite: `ssh mac 'cd /Users/natemccoy/rust/cargo-liner-berth-ceiling && zsh -lc "cargo nextest run -p cargo-berth --no-fail-fast; echo nextest-exit=\$?"' > <session dir>/mac-berth.log 2>&1`. Run it in the background.
   - `zsh -lc` gives TMPDIR as `/var/folders/…`, which is the case the brief names.
   - Read the `Summary` line and `nextest-exit=`. The ssh exit status gave a wrong answer on 2026-10-14, so do not use it.
5. After the checkpoint, remove the Mac clone: `ssh mac 'rm -rf /Users/natemccoy/rust/cargo-liner-berth-ceiling'`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-berth`: green.
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-berth`: green on natedev, and each new test runs under 0.1 s.
- Mac lane: `nextest-exit=0`, and the `Summary` shows every test passed, the 2 former failures and the 4 new tests included. An intermittent `LEAK` flag is accepted while every test passes and `nextest-exit=0` (showrunner, 2026-10-14): item 6 measured it as a race in nextest 0.9.136 on macOS, fixed in 0.9.145 (PR 3553), and the Mac upgrade went to natedev.
- The checkpoint notice gives the Mac `Summary` line with its wall time next to natedev's suite wall time, the TMPDIR route (`zsh -lc`, `/var/folders`), the candidate's sha256 and the leak's cause.

**Constraints from prior phases:** none. This is the only phase. `berth-hook-hang-unit` merged 50fdf062 and did not touch `worktree_context.rs`. The last change to that file was `berth-flake` phase 2 (d09f44c1), which added ceilings.
