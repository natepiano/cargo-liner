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

#### As-built

- `impl FromIterator<PathBuf> for DiscoveryCeilings` reads entries in order: a relative entry is dropped wherever it sits; an absolute entry before the first empty entry is replaced by its `fs::canonicalize` result, or kept as given when canonicalization fails; the first empty entry ends canonicalization, and every later absolute entry is kept as given. It is a `take_while` over a `by_ref` iterator, then an `extend` with the remaining absolute entries.
- `impl From<&OsStr> for DiscoveryCeilings` parses the path list through `env::split_paths`; non-UTF-8 handling is unchanged.
- The rules follow git's `setup.c` `canonicalize_ceiling_entry`, with one difference: git drops an entry that fails to canonicalize, cargo-berth keeps it as given. Such an entry cannot match a canonical ancestor of an existing directory, so it bounds nothing and never blocks the other entries.
- Invariant: `DiscoveryCeilings(Vec<PathBuf>)` holds ceiling directories, canonical except entries after an empty entry and entries that fail to canonicalize.
- Module tests `a_ceiling_named_through_a_symlink_stops_the_walk`, `a_ceiling_after_an_empty_entry_is_kept_as_given`, `a_ceiling_that_cannot_canonicalize_is_kept_as_given` and `a_relative_ceiling_is_dropped` read the private tuple field directly. `the_invocation_directory_is_examined_even_at_the_ceiling` and `a_ceiling_at_the_invocation_directory_does_not_stop_the_walk` compare `context.repository_root()` with `fs::canonicalize(repository.path())`.

**Files:**
- `crates/cargo-berth/src/ledger/worktree_context.rs` — `DiscoveryCeilings` impls, doc comments, module tests
- `docs/cargo-berth/as-built/test-git-and-discovery.md` — discovery section: ceiling rules, module-test list
- `docs/cargo-berth/as-built/worktree-coordination.md` — ceiling row
- `crates/cargo-berth/CHANGELOG.md` — Unreleased/Changed entry on `GIT_CEILING_DIRECTORIES` parsing

**Gotchas:**
- macOS tempdirs live under `/var`, a symlink to `/private/var`; a test comparing a discovered root with a tempdir path canonicalizes the tempdir path first.
- cargo-berth has no lib target; its unit tests run under `--bins`.
- The intermittent nextest `LEAK` on `answers a_defer_answer_adds_no_blocking_holder` on macOS is nextest 0.9.136's capture-pipe race (a child inherits a sibling test's stdout pipe), fixed in nextest 0.9.145 (PR 3553); a Mac run counts as passing when every test passes and nextest exits 0.
- Over ssh to the Mac the remote exit status is unreliable; read the nextest `Summary` line and an explicitly echoed exit code.

**Ruled out:**
- Dropping a ceiling that fails to canonicalize, as git does — kept as given, since it bounds nothing.
- A cargo-berth workaround for the macOS `LEAK` — the cause is in nextest, fixed upstream in 0.9.145.
