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

### Phase 2 — The watcher skips target/ and .git/objects  · status: done

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

### Phase 3 — cargo-port draws a frame only when something changed  · status: todo

**Depends on:** none

#### Work Order

**Goal:** an idle cargo-port uses under 1% of one core and draws no frame when nothing visible changed. During builds it stays under 3% of one core and under 2 frames each second.

**Source:** showrunner, 2026-10-28; the user ranks CPU work above all other work. Measured on natedev at 11:12–11:25: the cargo-port main thread used 14% of one core; perf put 90% of it in `Path == Path` compares from `RootItem::at_path` (`crates/cargo-port/src/project/root_item.rs`), about 4 frames each second at about 30 ms each. Notes: `/tmp/claude-1000/-home-natepiano-rust-cargo-liner/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/scratchpad/cpu_investigation.md`, section "cargo-port: a full frame".

**Constraints:**
- The event loop (`tui/terminal/event_loop.rs`, the `loop` in its run function) calls `ensure_visible_rows_cached`, `ensure_disk_cache`, `ensure_fit_widths_cached`, `ensure_detail_cached` and `draw_frame` on every wakeup. It wakes on input, on each background message, on each CPU pane sample, at least each second (`IDLE_HEARTBEAT`, `tui/app/mod.rs` ≈437), and each 80 ms while a lint, clean or run is live (`ANIMATION_TICK`).
- `ensure_detail_cached` (`tui/app/navigation/cache.rs`) already skips work with a `DetailCacheKey` of the selected row and `scan.generation()`. Use the same pattern: a typed key of the inputs, not a bool flag set by hand at call sites.
- `ProjectList::at_path` and its siblings (`tui/project_list/list.rs` ≈325 onward) search all roots and their members in a line. `compute_disk_cache` (`tui/panes/project_list/disk.rs` ≈172) calls `visible_row_disk_usage` for each visible row, which calls `at_path`, and clones the sorted vector once for each child node. The render calls `at_path` per row (`tui/panes/project_list/tree_rows.rs` ≈393).
- A visible clock, spinner or animation still moves at its own rate; a frame for it is a change.
- The Deleted mark, dismiss and all row content stay the same; this phase changes when work runs, not what is drawn.

**Spec:**
1. **Draw on change.** The loop draws a frame only when input, a background message, a timer or an animation changed visible state since the last frame. The 1 s idle heartbeat draws nothing when nothing changed.
2. **Path index.** `ProjectList` keeps an index from project path to its place in the roots, rebuilt when the roots change, so `at_path` and its siblings, `visible_row_disk_usage` and the render's per-row lookup do no linear search.
3. **Disk and width caches.** `ensure_disk_cache` and `ensure_fit_widths_cached` recompute only when their inputs change (scan generation, visible rows, the settings they read), keyed like `ensure_detail_cached`. `compute_disk_cache` clones no vector per child node.
4. **Tests, red first, each under one second:** an idle loop iteration with no change draws no frame; a background message that changes a row draws one; a lookup by path returns the same project as the linear search for roots, members, worktrees and vendored packages; the disk cache does not recompute when nothing changed and does when a disk size changes.
5. **Live CPU measure:** CPU seconds over 60 s, idle (no input, no cargo running in the scratch roots), for the installed binary and the branch binary, with the method in `docs/cargo-port/as-built/overview.md`, after checking that no natedev build hold is in force. Record both numbers and the frames drawn per second (`CARGO_PORT` perf log, if the loop logs frames).
6. CHANGELOG: one line in `crates/cargo-port/CHANGELOG.md`.

**Files:**
- `crates/cargo-port/src/tui/terminal/event_loop.rs` — draw on change
- `crates/cargo-port/src/tui/app/mod.rs` — idle heartbeat
- `crates/cargo-port/src/tui/app/navigation/cache.rs` — cache keys
- `crates/cargo-port/src/tui/project_list/list.rs` — path index
- `crates/cargo-port/src/tui/panes/project_list/disk.rs` — disk cache without per-node clones
- `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs` — per-row lookup through the index
- `crates/cargo-port/src/project/root_item.rs` — only if the index needs a change there
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — specs 1–3 and 6; hub: `crates/cargo-port/src/tui/project_list/list.rs`.
- `test` — spec 4, in new test modules beside the changed files, registered by `impl`.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; the idle CPU of spec 5 under 1% of one core on the branch binary. The checkpoint adds a Change-Review line.

### Phase 4 — A Deleted row stays on the list and shows its mark  · status: todo

**Depends on:** none

#### Work Order

**Goal:** a project marked Deleted stays in the list, struck through, through a full rescan and under `NO_COLOR=1`, until the user dismisses it. Two other display defects that the same live check sees are fixed with it: the sccache popup's right padding and a Git pane value that loses a letter from its middle.

**Source:** showrunner, 2026-10-28, from three reports:
- `App::rescan` (`tui/app/async_tasks/tree.rs` ≈52–58) clears every root, so a Deleted row leaves the list on a full rescan. This breaks the user rule "the row never leaves the list by itself" (found in the context map phase).
- Under `NO_COLOR=1`, cargo-port writes SGR 9 (strike-through) and then `ESC[;m` (crossterm `SetColors` with no colours), which resets the strike-through, so a Deleted row looks normal. Evidence: `/tmp/claude/unit/be6481c9-047e-4add-83c1-870d55a33343/phases/39/live/terminal_env_clean/`.
- Style's live shots (`/tmp/claude/unit/b063796c-82d6-4693-bea2-342456902a3a/phases/67/shots/after2`): at 160 columns the sccache popup's widest body row ("Cache location … ~/.cache/sccache") has one column of padding on the left and none on the right; at 80 columns the Git pane shows "fleet" as "flet". Both were in the code before style's change.

**Constraints:**
- Build the rescan fix with a type, not a guard: the rescan keeps each Deleted entry in its root as its own state until `x` (or the close box) dismisses it. A fresh scan that does not find the project does not drop that state.
- Under `NO_COLOR`, write text attributes (strike-through, bold, dim) without colour codes; never send a colour reset that also resets attributes.
- The sccache popup width is the widest row plus the left and right padding (`tui/sccache/render.rs`), so both sides get the same padding.
- For "fleet" → "flet", find why one letter goes from the middle of the word first. Where a value must be shortened, shorten it at its end with an ellipsis.

**Spec:**
1. **Rescan keeps Deleted rows.** After a full rescan, a project marked Deleted is still in its root, still struck through, and still dismissable; dismiss removes it as before.
2. **Strike-through under NO_COLOR.** With `NO_COLOR=1`, a Deleted row's bytes carry SGR 9 and no later reset clears it on that row.
3. **sccache popup padding.** The popup's widest row has the same padding on the left and the right at 160 columns.
4. **Git pane values.** At 80 columns, "fleet" shows whole, or shortened at its end with an ellipsis when the column cannot hold it; no letter goes from the middle of a value.
5. **Tests, red first, each under one second:** mark a row Deleted, run a full rescan, and the row is still there with the mark; a Deleted row rendered under `NO_COLOR` keeps the strike-through attribute in the output bytes; the sccache popup at 160 columns has equal left and right padding on its widest row; the Git pane at 80 columns shows the "fleet" value whole or end-shortened.
6. **Live check and shots** (unit director, after checking that no natedev build hold is in force): one live check with a scratch project deleted while cargo-port runs, then a full rescan, then `NO_COLOR=1`; shots from a real window at normal size on this unit's port: the Deleted row after a rescan, the Deleted row under `NO_COLOR=1`, the sccache popup at 160 columns, the Git pane at 80 columns.
7. CHANGELOG: one line for each of the four fixes in `crates/cargo-port/CHANGELOG.md`.

**Files:**
- `crates/cargo-port/src/tui/app/async_tasks/tree.rs` — rescan keeps Deleted entries
- `crates/cargo-port/src/tui/project_list/` — the Deleted entry state, if the root type holds it
- `crates/cargo-port/src/tui/terminal/` — NO_COLOR attribute output
- `crates/cargo-port/src/tui/sccache/render.rs` — popup width
- `crates/cargo-port/src/tui/panes/` — the Git pane value fit, once the cause is found
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — specs 1–4 and 7; hub: `crates/cargo-port/src/tui/app/async_tasks/tree.rs`.
- `test` — spec 5, in new test modules beside the changed files, registered by `impl`.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; the live check and the four shots of spec 6, with a design check pass. The checkpoint adds a Change-Review line for each visible fix.
