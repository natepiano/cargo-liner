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

#### As-built

- cargo-port adds one non-recursive inotify watch per folder from a lazy walk of each root (`watcher/roots.rs`, `directory_walk`, `watch_directories`, `register_directory_tree`). It puts no watch in `target/`, in a Cargo target directory it knows (configured and canonical path), or in `.git/objects`. `RegisteredRoots` records each watched folder.
- `WatchSkipSet` (`watcher/skip.rs`) is the one typed skip value; `skips_resolved` also checks the canonical path, so a symlinked target is skipped. Cargo-home watches go through the same skips.
- A folder created later gets watches for itself and its subfolders unless it is in a skipped tree; a removed or renamed folder drops its watches (`update_directory_watches`, `remove_directory_watches`); a newly known target directory prunes its watches (`replace_skip_set`).
- The target size reaches the project by a 30 s disk poll (`watcher/refresh.rs`, `DiskProbeScope`: `ProjectTree`, `TreeAndLinkedTarget`, `TreeAndExternalTarget`), not by `target/` events. A dismissed row stays dismissed through zero-byte and late positive poll replies (`tui/app/async_tasks/disk_handlers.rs`).
- With the user's roots on natedev the watch count dropped from about 438,000 to 90,266.

**Files:**
- `crates/cargo-port/src/watcher/skip.rs` — `WatchSkipSet`
- `crates/cargo-port/src/watcher/roots.rs` — folder walk, `RegisteredRoots`, watch updates, Cargo-home watch
- `crates/cargo-port/src/watcher/runtime.rs` — skips at startup and in the loop; the 30 s disk-poll clock
- `crates/cargo-port/src/watcher/refresh.rs` — `DiskProbeScope` disk probes
- `crates/cargo-port/src/tui/app/async_tasks/disk_handlers.rs` — Dismissed kept across poll replies
- `crates/cargo-port/src/watcher/skip_tests.rs`, `*_repair_tests.rs`, `repair_test_support.rs` — tests

**Binds later work:** the phase "cargo-port watches only project folders and the folders that lead to them" changes which folders the walk visits and keeps `WatchSkipSet` and the disk poll.

**Gotchas:** the disk column updates up to 30 s after a build. Run `verify.sh build` from the worktree; from another folder it fails with a JSON decode error.

### Phase 2b — cargo-port watches only project folders and the folders that lead to them  · status: todo

**Depends on:** Phase 2

#### Work Order

**Goal:** cargo-port puts inotify watches only inside its projects and on the folders between a watch root and a project, so the watch count follows the number of projects, not the number of folders under the roots.

**Source:** showrunner ruling, 2026-10-28 ~21:40 ("phase 2b is current. Watch only project folders and their parents for discovery, and put 2b before phase 3"). Phase 2 live count on natedev with the user's roots (`/etc`, `/tmp`, `~/.claude`, `~/rust`, `include_non_rust = true`): the branch binary held 90,266 watches, one per folder outside `target/` and `.git/objects` (`/tmp` 64,125, `~/rust` 20,632, `~/.claude` 5,421, `/etc` 129). Most of the `/tmp` folders are scratch and session trees that hold no project. The debug branch binary also used about 9 cores for the first 40 s of the folder walk.

**Constraints from prior phases:**
- Phase 2 put registration in `watcher/roots.rs`: a lazy folder walk (`directory_walk`, `watch_directories`) adds one non-recursive watch per folder; `register_directory_tree` watches a created folder and its subfolders; `update_directory_watches` and `remove_directory_watches` follow create, remove and rename. `RegisteredRoots` (`roots.rs`) records each watched folder. `WatchSkipSet` (`watcher/skip.rs`) is the one typed skip value: `target/`, `.git/objects`, and known Cargo target directories, canonical paths included. Keep all of it; this phase changes which folders the walk visits, not how a watch is added or skipped.
- The target size reaches the project by the 30 s disk poll (`watcher/refresh.rs`, `DiskProbeScope`), not by events. No change.
- Events inside a project feed lint triggers, the cargo metadata refresh, the git refresh and the disk refresh (`watcher/events.rs` `handle_notify_event`). Events outside every project feed discovery: `probe::project_level_dir` walks up to the project level and `probe_new_projects` probes the folder after `NEW_PROJECT_DEBOUNCE` (`watcher/probe.rs`). A project is a folder with `Cargo.toml`, or with `.git` when `include_non_rust` is on (`probe_project`).
- A deleted project still shows struck through (`Visibility::Deleted`) until the user dismisses it; the zero-byte `DiskUsage` reply for a missing folder must still reach the app.

**Spec:**
1. **Watched folder roles, by type.** Every watched folder has one role held in `RegisteredRoots` as an enum (for example `WatchedFolderRole`): `Project` (a project root and every folder in it, with the phase 2 skips), `Discovery` (a configured watch root, and each folder on the path from a watch root to a known project's parent), `Candidate` (a folder created in a `Discovery` folder, watched non-recursively while the probe decides), and `ConfigSource` (folders cargo-port reads outside projects: the Cargo home watch from `register_cargo_home_watch`, each ancestor `.cargo/` folder that `refresh.rs` (≈302) reads for `config.toml`, and the Git folder of a linked worktree that lives outside its project). A folder with no role gets no watch. No bool, and no path-prefix check at call sites, decides a role.
2. **Startup handoff.** `WatcherLoopStartup::register` (`runtime.rs` ≈158) runs before the scan delivers projects (`handle_scan_result`, `tui/app/async_tasks/dispatch.rs` ≈39, sends `WatcherMsg::Register`). Make it a typed sequence: (a) watch each configured root as `Discovery`; (b) on the scan's project list, watch each project's ancestors as `Discovery` and each project tree as `Project`; (c) then compare what the scan saw with the folders now watched, and probe any project folder made or removed between the scan and its watches. Buffered events are replayed after (b). The watcher reports ready only after (c). No walk visits a tree that holds no known project.
3. **Discovery routing.** `probe::project_level_dir` (`probe.rs` ≈154) takes the configured watch roots as the project-level boundary. Keep that list as configured roots only; a new `Discovery` or `Candidate` folder never joins it. A project made directly in a configured root, or in a `Discovery` folder, is found: event → candidate → probe → promotion to `Project`.
4. **Candidates are short-lived.** A folder created in a `Discovery` folder gets one non-recursive `Candidate` watch. When the probe (after `NEW_PROJECT_DEBOUNCE`) finds a project there, the folder becomes `Project` and its tree gets the phase 2 watches. When it finds none, the watch is removed; a folder made inside a `Candidate` gets no watch. So folder churn with no project (for example new session folders in `/tmp`) leaves no watch behind.
5. **Promotion and skip updates keep the roles.** `register_project_watch_if_needed` (`runtime.rs` ≈498) stops treating "under a registered root" as covered; it registers by role. `replace_skip_set` (`roots.rs` ≈216) re-applies skips only to `Project` trees and never walks a `Discovery` folder's other children.
6. **Removal.** A removed `Project`, `Discovery` or `Candidate` folder drops its watches. A removed project, and each project under a removed ancestor, still sends the zero-byte `DiskUsage` reply, so the row shows `Deleted` until the user dismisses it.
7. **Known limit, stated.** A project made inside an existing folder with no role is found by the next full rescan, not by a watch. Say this in the CHANGELOG line.
8. **Tests, red first, each under one second:** (a) a root with a project two levels down and a sibling tree with no project watches the root, the folder between and the project tree, and nothing in the sibling tree; (b) startup: a project made after the scan list and before its watches is found by step (c); (c) a folder created in a `Discovery` folder gets a `Candidate` watch, a project made in it is promoted to `Project`, a project made directly in a configured root is probed at the root level, not at its `Cargo.toml`; (d) a `Candidate` with no project loses its watch, and a folder made inside it gets none; (e) a late target-directory change prunes the target and adds no watch to a sibling tree; (f) an ancestor `.cargo/config.toml` change and a linked-worktree Git change still reach their refresh; (g) removing a project, and removing an ancestor of two projects, sends a zero-byte reply for each project. Call the registration, event and probe functions directly; no live inotify and no timing sleeps.
9. **Live count:** after checking that no natedev build hold is in force, run the branch binary (release build if `verify.sh build` offers one; else debug, and say so) from the worktree, with a scratch `XDG_CONFIG_HOME` holding a copy of the user's `~/.config/cargo-port/config.toml` with a scratch `[cache] root`. After 40 s count the inotify lines in `/proc/<pid>/fdinfo/<inotify fd>` and the CPU seconds. Phase 2 gave 90,266 and the installed binary about 438,000.
10. CHANGELOG: one line in `crates/cargo-port/CHANGELOG.md`.
11. **Test file names.** Phase 2 left `refresh_repair_tests.rs`, `roots_repair_tests.rs`, `skip_repair_tests.rs` and `repair_test_support.rs` in `watcher/`; "repair" is process history. Rename each for what it tests (for example `skip_alias_tests.rs`) and give `repair_test_support.rs` a name for what it provides.

**Files:**
- `crates/cargo-port/src/watcher/roots.rs` — folder roles, startup registration, new-folder handling
- `crates/cargo-port/src/watcher/runtime.rs` — startup handoff, role-aware registration, promotion
- `crates/cargo-port/src/watcher/probe.rs` — candidate probe result, promotion
- `crates/cargo-port/src/watcher/events.rs` — event routing by role
- `crates/cargo-port/src/watcher/mod.rs` — test module registration
- `crates/cargo-port/src/watcher/refresh_repair_tests.rs` — renamed (spec 11)
- `crates/cargo-port/src/watcher/roots_repair_tests.rs` — renamed (spec 11)
- `crates/cargo-port/src/watcher/skip_repair_tests.rs` — renamed (spec 11)
- `crates/cargo-port/src/watcher/repair_test_support.rs` — renamed (spec 11)
- `crates/cargo-port/src/watcher/refresh.rs` — `#[path]` line of a renamed test module; ancestor `.cargo/` reads
- `crates/cargo-port/src/watcher/skip.rs` — `#[path]` line of a renamed test module
- `crates/cargo-port/src/tui/app/async_tasks/dispatch.rs` — only if the scan must send its project list to the watcher as one message (spec 2)
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — specs 1–7, 10 and 11; hub: `crates/cargo-port/src/watcher/roots.rs`. All of `watcher/` except the tester's file.
- `test` — spec 8, in a new test module `crates/cargo-port/src/watcher/watch_role_tests.rs`, registered by `impl` in `watcher/mod.rs`.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; the live count of spec 9 is far under 90,266 and near the folder count of the user's projects plus their discovery folders. The checkpoint adds a Change-Review line.

### Phase 3 — cargo-port draws a frame only when something changed  · status: todo

**Depends on:** Phase 2b

#### Work Order

**Goal:** an idle cargo-port uses under 1% of one core and draws no frame when nothing visible changed. During builds it stays under 3% of one core and under 2 frames each second.

**Source:** showrunner, 2026-10-28; the user ranks CPU work above all other work. Measured on natedev at 11:12–11:25: the cargo-port main thread used 14% of one core; perf put 90% of it in `Path == Path` compares from `RootItem::at_path` (`crates/cargo-port/src/project/root_item.rs`), about 4 frames each second at about 30 ms each. Notes: `/tmp/claude-1000/-home-natepiano-rust-cargo-liner/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/scratchpad/cpu_investigation.md`, section "cargo-port: a full frame".

**Constraints:**
- The event loop (`tui/terminal/event_loop.rs`, the `loop` in its run function) calls `ensure_visible_rows_cached`, `ensure_disk_cache`, `ensure_fit_widths_cached`, `ensure_detail_cached` and `draw_frame` on every wakeup. It wakes on input, on each background message, on each CPU pane sample, at least each second (`IDLE_HEARTBEAT`, `tui/app/mod.rs` ≈437), and each 80 ms while a lint, clean or run is live (`ANIMATION_TICK`).
- `ensure_detail_cached` (`tui/app/navigation/cache.rs`) already skips work with a `DetailCacheKey` of the selected row and `scan.generation()`. Use the same pattern: a typed key of the inputs, not a bool flag set by hand at call sites. That generation alone is not a full key: `update_generations_for_msg` (`tui/app/async_tasks/dispatch.rs` ≈32) leaves it unchanged for an unselected project's `DiskUsage`, and `ProjectStorage` (`scan/background.rs` ≈258) does not touch it. Each cache names its own inputs: list rows, disk totals, column widths and storage. A new cache state is an enum (for example `NotComputed` / `Current(key)`), not a bare `Option<Key>`.
- Phase 2 made the target size arrive by a 30 s disk poll. A poll reply that changes no size must not draw a frame; one that changes an unselected project's size must update its row.
- A root path lookup already uses `IndexMap::get`; `ProjectList::at_path` and its siblings (`tui/project_list/list.rs` ≈325 onward) still search descendants (members, worktrees, vendored packages) in a line. The index must stay valid after an insert, a replace, a regroup and a rescan (`tui/project_list_state.rs`, `tui/app/tree_mutation.rs`). `compute_disk_cache` (`tui/panes/project_list/disk.rs` ≈172) calls `visible_row_disk_usage` for each visible row, which calls `at_path`, and clones the sorted vector once for each child node. The render calls `at_path` per row (`tui/panes/project_list/tree_rows.rs` ≈393).
- A visible clock, spinner or animation still moves at its own rate (`App::animation_timeout`, `tui/app/mod.rs` ≈436, 80 ms); a frame for it is a change. The build budget of under 2 frames each second applies while cargo runs in a project and no spinner or animation is on screen.
- The event loop logs only slow frames today, so frame rate needs a counter of drawn frames (for example a perf-log line each second with the count), not a count of slow-frame lines.
- The Deleted mark, dismiss and all row content stay the same; this phase changes when work runs, not what is drawn.
- Phase 2's full suite on natedev had two tests over one second: `tui::app::tests::interaction::project_list_renders_framework_overflow_affordance` (1.141 s) and `tui::app::tests::interaction::selection_row_highlight_covers_ansi_colored_log_text` (1.081 s). Both draw frames; this phase brings each under one second.

**Spec:**
1. **Draw on change.** The loop draws a frame only when input, a background message, a timer or an animation changed visible state since the last frame. The 1 s idle heartbeat draws nothing when nothing changed.
2. **Path index.** `ProjectList` keeps an index from project path to its place in the roots, rebuilt when the roots change, so `at_path` and its siblings, `visible_row_disk_usage` and the render's per-row lookup do no linear search.
3. **Disk and width caches.** `ensure_disk_cache` and `ensure_fit_widths_cached` recompute only when their inputs change (scan generation, visible rows, the settings they read), keyed like `ensure_detail_cached`. `compute_disk_cache` clones no vector per child node.
4. **Tests, red first, each under one second:** an idle loop iteration with no change draws no frame; a background message that changes a row draws one; a `DiskUsage` reply for an unselected project with a new size updates its row, and one with the same size draws nothing; the index finds the right project after insert, replace, regroup and rescan; a lookup by path returns the same project as the linear search for roots, members, worktrees and vendored packages; the disk cache does not recompute when nothing changed and does when a disk size changes.
5. **Live CPU measure:** CPU seconds over 60 s, idle (no input, no cargo running in the scratch roots), for the installed binary and the branch binary, with the method in `docs/cargo-port/as-built/overview.md`, after checking that no natedev build hold is in force. Record both numbers and the frames drawn per second from the frame counter. Then the same with a `cargo build` running in a scratch project and no spinner on screen.
6. CHANGELOG: one line in `crates/cargo-port/CHANGELOG.md`.

**Files:**
- `crates/cargo-port/src/tui/terminal/event_loop.rs` — draw on change
- `crates/cargo-port/src/tui/app/mod.rs` — idle heartbeat; the two slow interaction tests (≈7200, ≈8238)
- `crates/cargo-port/src/tui/app/navigation/cache.rs` — cache keys
- `crates/cargo-port/src/tui/project_list/list.rs` — path index
- `crates/cargo-port/src/tui/panes/project_list/disk.rs` — disk cache without per-node clones
- `crates/cargo-port/src/tui/panes/project_list/tree_rows.rs` — per-row lookup through the index
- `crates/cargo-port/src/project/root_item.rs` — only if the index needs a change there
- `crates/cargo-port/src/tui/project_list_state.rs` — index upkeep on change
- `crates/cargo-port/src/tui/app/tree_mutation.rs` — index upkeep on insert, replace, regroup
- `crates/cargo-port/src/tui/app/async_tasks/dispatch.rs` — generation inputs for the new keys
- `crates/cargo-port/src/tui/terminal/event_loop_tests.rs` — new; tester
- `crates/cargo-port/src/tui/project_list/path_index_tests.rs` — new; tester
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — specs 1–3 and 6, the frame counter, and the two slow interaction tests in `tui/app/mod.rs`; hub: `crates/cargo-port/src/tui/project_list/list.rs`. Not the tester's two files.
- `test` — spec 4, in `tui/terminal/event_loop_tests.rs` and `tui/project_list/path_index_tests.rs`, registered by `impl`.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; the idle CPU of spec 5 under 1% of one core on the branch binary; during the build, under 3% of one core and under 2 frames each second with no spinner on screen; the two interaction tests each under one second. The checkpoint adds a Change-Review line.

### Phase 4 — A Deleted row stays on the list and shows its mark  · status: todo

**Depends on:** Phase 3

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
1. **Rescan keeps Deleted rows.** After a full rescan completes, a project marked Deleted is still in its root, still struck through, and still dismissable; dismiss removes it as before. Keeping entries when `rescan()` starts is not enough: `handle_scan_result` (`tui/app/async_tasks/dispatch.rs` ≈39) later replaces the whole list, and the Dismissed guard for late disk replies (`disk_handlers.rs` ≈65) reads the kept entry's visibility.
2. **Strike-through under NO_COLOR.** With `NO_COLOR=1`, a Deleted row's bytes carry SGR 9 and no later reset clears it on that row.
3. **sccache popup padding.** The popup's widest row has the same padding on the left and the right at 160 columns.
4. **Git pane values.** At 80 columns, "fleet" shows whole, or shortened at its end with an ellipsis when the column cannot hold it; no letter goes from the middle of a value.
5. **Tests, red first, each under one second:** mark a row Deleted (a plain project, a workspace member in a group, and a worktree), run a full rescan through to its completed scan result, and the row is still there with the mark; then dismiss it, and a late zero-byte and a late positive disk reply do not bring it back; a Deleted row rendered under `NO_COLOR` keeps the strike-through attribute in the output bytes; the sccache popup at 160 columns has equal left and right padding on its widest row; the Git pane at 80 columns shows the "fleet" value whole or end-shortened.
6. **Live check and shots** (unit director, after checking that no natedev build hold is in force): one live check with a scratch project deleted while cargo-port runs, then a full rescan, then `NO_COLOR=1`; shots from a real window at normal size on this unit's port: the Deleted row after a rescan, the Deleted row under `NO_COLOR=1`, the sccache popup at 160 columns, the Git pane at 80 columns.
7. CHANGELOG: one line for each of the four fixes in `crates/cargo-port/CHANGELOG.md`.

**Files:**
- `crates/cargo-port/src/tui/app/async_tasks/tree.rs` — rescan keeps Deleted entries
- `crates/cargo-port/src/tui/app/async_tasks/dispatch.rs` — the completed scan result keeps Deleted entries
- `crates/cargo-port/src/tui/app/async_tasks/disk_handlers.rs` — late replies after dismiss
- `crates/cargo-port/src/tui/app/async_tasks/deleted_rescan_tests.rs` — new; tester
- `crates/cargo-port/src/tui/terminal/no_color_tests.rs` — new; tester
- `crates/cargo-port/src/tui/project_list/` — the Deleted entry state, if the root type holds it
- `crates/cargo-port/src/tui/terminal/` — NO_COLOR attribute output
- `crates/cargo-port/src/tui/sccache/render.rs` — popup width
- `crates/cargo-port/src/tui/panes/` — the Git pane value fit, once the cause is found
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer + 1 tester.
- `impl` — specs 1–4 and 7; hub: `crates/cargo-port/src/tui/app/async_tasks/tree.rs`. Not the tester's files.
- `test` — spec 5, in `tui/app/async_tasks/deleted_rescan_tests.rs` and `tui/terminal/no_color_tests.rs`, plus the sccache and Git pane cases in test modules beside those files, each named in the tester's first board post; `impl` registers them.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; the live check and the four shots of spec 6, with a design check pass. The checkpoint adds a Change-Review line for each visible fix.
