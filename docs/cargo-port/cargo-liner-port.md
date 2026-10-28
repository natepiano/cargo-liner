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

## Delegation Context

- **Project:** cargo-port — a TUI for inspecting and managing Rust projects, built on `tui_pane`
- **Project started:** 2026-10-28T14:44:58.152+00:00
- **Stack:** Rust 2024; ratatui, crossterm, tokio, reqwest, notify; `tui_pane` (workspace member)
- **Layout:** `crates/cargo-port/src/` (app source), `crates/cargo-port/tests/`, `crates/cargo-port/themes/`, `docs/cargo-port/as-built/` (as-built docs)
- **Key files:** `crates/cargo-port/src/main.rs` — entry; `crates/cargo-port/src/tui/app/construct.rs` — App construction; `crates/cargo-port/src/watcher/` — file watcher; `crates/cargo-port/src/tui/app/dismiss.rs` — dismiss; `crates/cargo-port/README.md` — crate readme
- **Test lanes:** cargo-port: `crates/cargo-port/tests/`
- **Build:** `bash ~/.claude/scripts/direct/verify.sh check cargo-port`
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test cargo-port`
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-port`
- **Invariants:** no test takes a second; a deleted project stays in the list, struck through, until the user dismisses it.

## Phases

### Phase 1 — A context map of cargo-port  · status: done

**Depends on:** none

#### As-built

`docs/cargo-port/as-built/overview.md` is the map of cargo-port that every cargo-port phase reads first. It has a module map for each top-level module and each `tui/` submodule; data flows for startup, scan to project list, the watcher (deletion, `Visibility::Deleted` and dismiss), async tasks, GitHub and CI enrichment, lint and sccache, and the render loop; state ownership; configuration, themes and keymaps; the test layout and helpers; and the live check method (a scratch root with an absolute scratch `XDG_CONFIG_HOME`), with the `~/rust/repro` scratch repros next to it. It links `targets-pane.md` and `lint-file-lock-spinner.md`. Each backticked path in it exists.

**Files:**
- `docs/cargo-port/as-built/overview.md` — the cargo-port map

**Binds later work:** later cargo-port Work Orders cite the overview's live check method instead of a new survey.

**Gotchas:** `App::rescan` (`tui/app/async_tasks/tree.rs`) clears every root, so a Deleted row leaves the list on a full rescan; the overview names this as the one exception to "the row never leaves the list by itself", and the phase "A Deleted row stays on the list and shows its mark" removes it.

### Phase 2 — The watcher skips target/ and .git/objects  · status: todo

**Depends on:** none

#### Work Order

**Goal:** cargo-port does not put inotify watches inside any `target/` tree or `.git/objects/`, so its watch count stays small and a removed build folder costs no CPU. Folders made later still get a watch unless they are inside a skipped tree.

**Source:** showrunner, 2026-10-28 (urgent). builds-unit measured cargo-port with perf at 11:28–11:40 AST: pid 2789733 held 432,930 inotify watches (83% of the 524,288 limit), because it watches every root recursively, `target/` included (`watcher/roots.rs` ≈91, `watcher/runtime.rs` ≈502). On each removed watched folder, notify 9.0.0-rc.4 (`inotify.rs` ≈715, ≈769) runs `starts_with` over every watched path; builds remove many folders under `target/`. Cost: up to 2 cores, 34% of samples in `Path::_starts_with`; rust-analyzer and editors risk ENOSPC. Perf data: `/tmp/claude-1000/-home-natepiano-worktrees-claude-build-followups-builds/51505394-82db-43d3-a528-136f4ee38d7a/scratchpad/portd.data`.

**Constraints:**
- notify has no exclude list. Register watches one folder at a time (non-recursive) and skip the excluded trees; on a create event for a folder, add a watch for it and its subfolders unless it is inside a skipped tree.
- Today `target/` events feed project state: build-directory events update disk usage, and target-layout paths call `spawn_project_refresh` (`watcher/events.rs`, `watcher/probe.rs`). Each consumer of a `target/` event gets its data another way: a slow poll of the target size (for example on the existing disk refresh timing), not a watch. Name each consumer and its replacement in the summary.
- A deleted project still shows struck through (`Visibility::Deleted`) until the user dismisses it; no change to that path.
- The skip set is one typed value (for example an enum of skipped tree kinds), not string checks spread over call sites.

**Spec:**
1. Root registration walks folders and adds one non-recursive watch per folder, skipping `target/` (and a Cargo `build.target-dir` the project names, when the code already knows it) and `.git/objects/`.
2. A folder created later under a watched folder gets watches for itself and its subfolders, unless it is inside a skipped tree. A removed folder drops its watch.
3. Every behavior that read `target/` events keeps working through a slow poll; the disk size of a project still updates after a build.
4. Tests, red first, each under one second: a root with `target/` and `.git/objects/` gets no watch inside them; a new `src/sub/` folder gets a watch; a new folder inside `target/` gets none; the target size still reaches the project after the poll.
5. Live check: run the built cargo-port against a scratch root (method in `docs/cargo-port/as-built/overview.md`) and against the default roots; record the watch count from `/proc/<pid>/fdinfo/<inotify fd>` inotify lines, before (installed binary) and after (branch binary).
6. CHANGELOG: one line in `crates/cargo-port/CHANGELOG.md`.

**Files:**
- `crates/cargo-port/src/watcher/` — registration, create/remove handling, skip set
- `crates/cargo-port/src/tui/app/async_tasks/disk_handlers.rs` — target size by poll; the disk poll owner the diagnosis names joins this list
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — specs 1–3 and 6; hub: `crates/cargo-port/src/watcher/mod.rs`.
- `test` — spec 4, in a separate test module under `crates/cargo-port/src/watcher/` registered by `impl`.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; the live watch counts of spec 5. The checkpoint adds a Change-Review line.
