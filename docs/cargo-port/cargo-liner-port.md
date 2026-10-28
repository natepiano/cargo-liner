# port

> **Production: cargo-liner** — unit `port-unit`; production doc `docs/cargo-port/cargo-liner-production.md`

## Source

2026-10-28 10:41 EDT

you need to create a new unit to handle all cargo port enhancements it should be called port (session name - i think your approach is to call it port-unit for its id) it should be the one to build up context about cargo-port and not hand work to other units such as handler that is port work

## Unit charter

- **Owns:** `crates/cargo-port/` and the port plans in `docs/cargo-port/`. Every cargo-port enhancement and fix comes to this unit; no other unit gets cargo-port work.
- **Shared crates:** when a cargo-port change needs a `tui_pane` change, this unit makes it and claims the file with `cargo-berth --before` the current holder, as earlier units did. It does not hand the change to the tile or handler unit.
- **In flight elsewhere:** handler phase 39 ("A project whose parent is deleted shows as deleted") was already running when this unit started, so handler finishes it. It is diagnosing a repro (a repo with a corrupt `.git/HEAD` whose parent folder was removed) that did not get the Deleted mark. Its findings come to this unit after it merges; any follow-up from it comes here.
- **Not this unit's:** cargo-port findings that the style production holds (for example proposal P265, CPU bands, quota mode and pagination states) stay with `cargo-liner-style`.
- **New work:** each request the showrunner sends becomes a new phase in this plan, with its own Work Order, after Phase 1.

## Phases

### Phase 1 — A context map of cargo-port  · status: todo

**Depends on:** none

#### Work Order

**Goal:** one as-built overview of cargo-port that a writer reads before any cargo-port phase. It names where each part of the app lives, how data moves from scan and watcher to the drawn rows, and how to test and check the app live. Later phases cite it instead of a new survey.

**Source:** user, 2026-10-28: this unit "should be the one to build up context about cargo-port". The crate has 293 source files (about 100,000 lines) and only two as-built docs (`targets-pane.md`, `lint-file-lock-spinner.md`), each for one feature.

**Spec:**
1. Write `docs/cargo-port/as-built/overview.md`. It describes the code as it is now, and it has no plan or phase text. Keep it short: it is a map for the next agent, not a full description (showrunner, 2026-10-28). Each entry names where to look, in one or two lines.
2. **Module map.** One entry for each top-level module under `crates/cargo-port/src/` (`cache_paths`, `channel`, `ci`, `config`, `constants`, `enrichment`, `http`, `lint`, `process_observation`, `project`, `scan`, `sccache`, `support`, `test_support`, `themes`, `tui`, `watcher`) and for each submodule of `tui/` (`app`, `panes`, `state`, `input`, `keymap`, `integration`, `terminal` and the others): its job, its main types, and its entry functions.
3. **Data flows,** each as a short numbered chain of named functions and types:
   - startup: config load, theme load, `App` construction (`tui/app/construct.rs`), first scan;
   - scan to project list: discovery, `RootItem` and list order, members and worktrees;
   - watcher: the events it reads (`watcher/events.rs`, `roots.rs`, `runtime.rs`, `refresh.rs`), and how each one reaches project state, disk usage and `Visibility` (`Deleted` included), and dismiss (`tui/app/dismiss.rs`). State the rule: a deleted project is struck through (`Visibility::Deleted`), the user dismisses it with `x` or the close box, and the row never leaves the list by itself;
   - async tasks: the handlers in `tui/app/async_tasks/` and the messages they handle;
   - GitHub and CI enrichment: `http/`, rate limits, `ci`;
   - lint runtime and sccache status;
   - the render loop and pane drawing (`tui/terminal/event_loop.rs`, `tui/render.rs`), and the line between cargo-port and `tui_pane`.
4. **State ownership.** Which struct owns each piece of state (`App`, each pane, the net state, the lint runtime), and which code changes it.
5. **Configuration and themes.** The config file, its sections and `normalize_config`; the theme files under `crates/cargo-port/themes/`; the keymap files.
6. **Tests.** The test layout (inline `mod tests` and separate `*_tests.rs`); the shared helpers (`make_app`, `make_app_with_config`, `count_allocations`, the seeded git repository in `test_support/git_seed`); the rule that no test takes a second; the commands `verify.sh lint cargo-port` and `verify.sh test cargo-port`.
7. **Live check method.** How to run the built cargo-port against a scratch root with an absolute scratch `XDG_CONFIG_HOME`, so the user's own config is not read or changed. Run it once in this phase: make a scratch project under the scratch root, start cargo-port, and see the project listed. Record the commands in the doc and the result in the phase report.
7a. Scratch repros: projects under `~/rust/repro` are change-review repros made by `scratch.py --visible`. Name this in the doc, next to the live check method.
8. Link the two feature as-built docs from the overview, and add one line to `crates/cargo-port/README.md` only if it already has a section for developer docs.
9. Every path, type and function the doc names exists at the phase's base commit. A check script in the phase report lists each backticked path in the doc and confirms it exists.

**Files:** `docs/cargo-port/as-built/overview.md` (new); `crates/cargo-port/README.md` only per spec 8. No source file changes.

**Workers:** 1 writer (`impl`), read-only on `crates/cargo-port/src/`.

**Acceptance gate:** the path check of spec 9 passes with no missing path; the live check of spec 7 shows the scratch project listed; a reviewer reads three flows of spec 3 against the code and finds no step that the code does not do. No build or test change, so no `verify.sh` run is needed beyond `verify.sh lint cargo-port` if the README changes.
