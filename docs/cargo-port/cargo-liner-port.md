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

### Phase 2m — On macOS cargo-port watches each root recursively again  · status: done

**Depends on:** Phase 2

#### As-built

- One typed backend choice, `WatchRegistration { RecursiveRoot, PerFolder }` (`watcher/roots.rs`), picked once where the watcher is made (`watcher/runtime.rs`, beside `RecommendedWatcher::new`): `RecursiveRoot` on macOS (FSEvents), `PerFolder` elsewhere (inotify). `RegisteredRoots` holds it.
- `registration_plan(root, skips, registration)` yields the paths and modes to watch: one recursive root for `RecursiveRoot`, the phase 2 lazy folder walk for `PerFolder`. `register_watch_roots_with_skips` takes the backend.
- Under `RecursiveRoot`, create, remove and rename events add or drop no watches (`update_directory_watches` returns early), `replace_skip_set` only swaps the skip set, and the Cargo-home watch is skipped when a root already covers it. Events in `target/` and `.git/objects` are still dropped by `filter_event_paths`.
- `PerFolder` is phase 2's behaviour, unchanged. Mac suite 1109/1109.

**Files:**
- `crates/cargo-port/src/watcher/roots.rs` — `WatchRegistration`, `registration_plan`, `filter_event_paths`
- `crates/cargo-port/src/watcher/runtime.rs` — the backend choice
- `crates/cargo-port/src/watcher/watch_registration_tests.rs` — registration tests for both backends

**Binds later work:** Phase 2b's folder roles apply only to `PerFolder`. Phase 2c removes a second recursive watch on a root that another root covers (review finding F001).

**Gotchas:** overlapping configured roots (`/r` and `/r/project`) still get two recursive watches on macOS until Phase 2c.

### Phase 2f — cargo-port passes clippy on macOS  · status: done

**As-built:** `InsufficientProcessIdentity::PlatformCreationTokenUnavailable` (`crates/cargo-port/src/process_observation/identity.rs`) is gated `cfg(not(target_os = "macos"))`, the only platform that constructs it. Before, it also existed in macOS test builds, where `dead_code` failed clippy under `-D warnings`. Each other variant's cfg matches the cfg of its constructors. Proof: macOS `cargo clippy -p cargo-port --all-targets -- -D warnings` passes; the identity tests pass on the Mac.

### Phase 2b — cargo-port watches only project folders and the folders that lead to them  · status: done

**Depends on:** Phase 2m

#### As-built

- On Linux (`PerFolder`) each watched folder has one `WatchedFolderRole` in `RegisteredRoots` (`watcher/roots.rs`): `Project`, `Discovery`, `Candidate` or `ConfigSource`. A folder with no project below it gets no watch. Live count with the user's roots: 27,811 watches (phase 2: 90,266; before: about 438,000).
- `role()` and `event_role()` return `Subscription::{Unwatched, Watched(WatchedFolderRole)}`.
- Startup registration, the check of folders made or removed between the scan and its watches, and the replay of buffered events are one typed sequence that ends in ready (`watcher/runtime.rs`, `complete_startup_registration`).
- A folder made in a `Discovery` folder gets one `Candidate` watch; the probe promotes it to `Project` or removes the watch. A folder that comes back gets the watches of its known projects again.
- External Git events (a linked worktree's `FETCH_HEAD` and `ORIG_HEAD`, a containing repository's `.git/config`) reach the Git refresh without a project-containment check (`watcher/events.rs`).
- macOS (`RecursiveRoot`) keeps one recursive watch per configured root.

**Files:**
- `crates/cargo-port/src/watcher/roots.rs` — roles, registration, recovery of known projects under a folder that comes back
- `crates/cargo-port/src/watcher/runtime.rs` — startup sequence
- `crates/cargo-port/src/watcher/events.rs` — event routing by role
- `crates/cargo-port/src/watcher/probe.rs` — candidate outcomes
- tests in `crates/cargo-port/src/watcher/`: `watch_role_tests.rs`, `discovery_event_tests.rs`, `subscription_recovery_tests.rs`, `linked_target_poll_tests.rs`, `cargo_home_watch_tests.rs`, `skip_alias_tests.rs`, `watch_test_support.rs`

**Binds later work:** Phase 2c uses the `Subscription` API; coverage follows successful subscriptions. Phase 3 measures idle CPU after the startup sequence is ready.

**Gotchas:** a `.cargo/` folder outside the configured roots that is deleted and made again gets its watch back only at the next rescan (CHANGELOG). A project made inside an existing folder with no role is found by the next full rescan.

### Phase 2d — A manifest cargo cannot read shows once on its row  · status: done

**As-built:** `CargoMetadataError::from` (`crates/cargo-port/src/scan/cargo_metadata.rs`) classifies a manifest that `cargo metadata` rejects as `ManifestRejected` with cargo's first error line. `Scan` owns `ManifestMetadataStates` (`tui/app/manifest_metadata.rs`), a per-manifest store that outlives one scan: each Rust project has a `ProjectMetadataState`, a failure records the rejected state, a later success clears it, and projects that are no longer Rust drop out. The row shows a red ` !` after the name; the Package pane shows the error (`clip_rows_with_ellipsis` in `tui/render.rs`, shared with the description). A rejected manifest makes no toast on any scan; launch, I/O and timeout failures keep their error toasts. The `WorkspaceMissing` suppression and startup completion accounting are unchanged. Live check on natedev: no toast, row marker, Package pane error, also after a rescan.

### Phase 2g — cargo-mend fixes a source mounted as a module in several places  · status: done

**As-built:** mend's visibility reduction (`crates/cargo-mend/src/compiler/persistence/visibility_constraint.rs`) groups each physical source by `SourceModuleMount` (package root plus module path), so library, binary and test compilations of one module are one mount. For a source with several mounts, an automatic edit (restricted annotation, unused pub, narrow to `pub(crate)`, field visibility, pub-use facade) lands only when every mount proposes the same `VisibilityReplacement`; a silent, unknown or disagreeing mount keeps the current annotation. Agreement runs on each source the edit changes, including the child declarations a parent facade fix rewrites (`PubUseNarrowingSite`, now in `schema.rs`). Manual diagnostics stay, once each. Findings schema 36. Tests: `tests/diagnostics/shared_source_visibility.rs` and two reduction tests.

### Phase 2e — The shortcuts overlay's wheel scroll lives in tui_pane  · status: done

**As-built:** `tui_pane` owns the mouse wheel over its framework overlays (checkpoint 27059d4c). `wheel_framework_overlay` (`crates/tui_pane/src/framework/wheel.rs`) scrolls the Settings, Keymap or GlobalShortcuts overlay under the pointer and returns `OverlayWheel`. Settings movement is one crate-private function in `app_settings/navigation.rs` that key navigation and the wheel both call; a host clears its own state first through `SettingsHost::before_settings_navigation`. cargo-port (`tui/input/dispatch.rs`) deleted its three overlay scroll arms and calls the function after it inverts the scroll; its remaining `FrameworkOverlayId` arms are close and key routing. Handler Phase 57 wires cargo-tile and cargo-handler to the function.

### Phase 2c — On macOS overlapping roots get one recursive watch  · status: done

**Depends on:** Phase 2b

#### Work Order

**Goal:** with the `RecursiveRoot` backend (macOS), a configured root that another configured root covers gets no second recursive watch.

**Source:** phase 2m contract review, finding F001 (minor). The showrunner moved it out of 2m on 2026-10-29: overlapping roots are a rare setup, and two recursive watches still cost far less than per-folder watches.

**Constraints:**
- Coverage follows successful subscriptions: `RegisteredRoots::covers()` (`roots.rs` ≈109) reads `dirs`, and recursive registration adds only paths it watched. Keep every configured root as a discovery boundary while removing duplicate watches, and add tests for both root orders that check the configured boundaries stay and that a failed parent registration still leaves the covered root watched.
- From phase 2b: `RegisteredRoots` keeps a role map; `role()` and `event_role()` return `Subscription::{Unwatched, Watched(WatchedFolderRole)}`.
- The invariant at `watcher/runtime.rs` ≈989 (its tests ≈1071) holds by construction: one notify watch per path; no `watch()` call lands on a path a recursive root covers.
- Under `RecursiveRoot`, register a recursive watch only for each root that no other configured root covers, in any root order (`/r` then `/r/project`, and `/r/project` then `/r`, both give one watch on `/r`). Each configured root stays its own discovery root (`RegisteredRoots.dirs` unchanged); only the watch set changes.
- `PerFolder` behaviour is unchanged.

**Spec:**
1. Covered-root removal in the `RecursiveRoot` registration plan (`roots.rs`, `registration_plan` / `register_watch_roots_with_skips`).
2. **Tests, red first, each under one second:** both root orders give one registered path; the invariant test at `runtime.rs` ≈1071 also runs with `RecursiveRoot`.
3. CHANGELOG: one line.

**Files:**
- `crates/cargo-port/src/watcher/roots.rs`
- `crates/cargo-port/src/watcher/runtime.rs` — the invariant test; the cargo home resolved once and passed to registration (showrunner, 2026-10-31: the Mac tests read the real `~/.cargo`)
- `crates/cargo-port/src/watcher/watch_registration_tests.rs`
- `crates/cargo-port/src/watcher/cargo_home_watch_tests.rs`, `skip_alias_tests.rs`, `watch_role_tests.rs` — a test cargo home
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer (`impl`), who also writes the tests.

**Acceptance gate:** `verify.sh test cargo-port` (Mac) and `verify.sh lint cargo-port` clean; no test takes a second.

### Phase 2h — mend keeps a re-export another package uses  · status: todo

**Why:** showrunner 2026-10-31, found during 2g. The subtree re-export planner groups facts and module mounts by crate root (`crates/cargo-mend/src/fixes/subtree_reexport/scan.rs` `resolved_crate_facts` ≈150, mount filter ≈165). When two packages mount one source with `#[path]` and both use a `pub use` in it, a fix planned for one package can delete the export that the other package still names. Same phase: two style nits left open in 2g.

**Depends on:** Phase 2g (mount identity `SourceModuleMount`).

#### Work Order

**Goal:** a subtree re-export fix changes a shared source only when no other package's mount still needs the re-export it removes.

**Constraints:**
- Fix at the cause: the planner sees every mount of a physical source, across packages, before it removes or moves a re-export. Use 2g's `SourceModuleMount` as the one mount identity; no second mount type.
- A source with one mount keeps today's behavior and output.
- No new flag or option.

**Spec:**
1. For a source mounted by several packages, a re-export edit lands only when every mount that uses the re-export gets the same edit; otherwise mend keeps the re-export and reports it once as manual.
2. F005: replace the bool accumulator `agreeing` in `resolve_visibility_proposals` (`compiler/persistence/visibility_constraint.rs`) with an enum that names the state.
3. F006: import `PubUseNarrowingSite` in `visibility_edit_sources` instead of the full `super::schema::PubUseNarrowingSite` path.
4. **Tests, red on main first, each under one second:** a fixture with two packages that `#[path]`-mount one source holding a `pub use`, where only one package's callers could be redirected; `mend --fix` keeps the re-export and both packages build.

**Files:**
- `crates/cargo-mend/src/fixes/subtree_reexport/scan.rs`, `crate_files.rs`, `mod.rs` — mount grouping
- `crates/cargo-mend/src/compiler/persistence/visibility_constraint.rs` — F005, F006
- `crates/cargo-mend/tests/` — the fixture test
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer (`review` slot, the 2g writer), who also writes the tests.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean; the in-repo `mend --fix` step reports no new error.

### Phase 2i — mend never writes the same import twice  · status: todo

**Why:** showrunner 2026-10-31. `mend --fix` on hana's `crates/hana_metrics/src/reading/tests.rs`, which has many `use crate::test_support::<item>;` imports, wrote both `use crate::test_support;` and `use crate::test_support::{self};`. The compile failed with E0252 and mend rolled the fix back. Log: `/tmp/claude-1000/-home-natepiano-worktrees-claude-natedev-builds/51505394-82db-43d3-a528-136f4ee38d7a/scratchpad/mend_dup_import.log`. Cost: one lint wait per hit; nothing is blocked.

**Depends on:** Phase 2g.

#### Work Order

**Goal:** the fixer cannot write an import that names something the module already imports.

**Constraints:**
- Fix at the cause, by type: the import edits for one module go through one owner that holds the module's import set, so a second spelling of the same name (`use a::b;`, `use a::b::{self};`) is one entry, not two lines. No text check after the fact.
- Find which fix (or which pair of fixes) writes each line, and name it in the report.
- Output for every other import shape stays the same.

**Spec:**
1. Any set of fixes that adds or rewrites imports in one module writes each imported name once.
2. **Tests, red on main first, each under one second where rustc allows:** a fixture module with several `use crate::m::<item>;` imports that mend rewrites; `mend --fix` leaves exactly one import of `m` and the crate builds.
3. CHANGELOG: one line.

**Files:**
- `crates/cargo-mend/src/fixes/` — the import edit owner (the writer names the files)
- `crates/cargo-mend/tests/` — the fixture test
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer (`review` slot, the mend writer), who also writes the tests.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean; the in-repo `mend --fix` step reports no new error.

### Phase 2j — a mend cell keeps its progress bar after a diagnostic  · status: todo

**Why:** showrunner 2026-10-31; the user asked why a cargo-tile mend cell shows no progress bar. Claim from the tile unit, not yet confirmed live: in Full mode the first forwarded diagnostic calls `progress.stop_for_forwarded_output()` (`src/compiler/build/stderr.rs:434`). That ends the progress state for the pass, so no counter reaches the capture log after it.

**Depends on:** none (runs beside 2h and 2i; separate files).

#### Work Order

**Goal:** a forwarded diagnostic block prints between progress updates; the progress counter continues to the end of the pass.

**Constraints:**
- Confirm the cause with a live run first, before any edit: a real `cargo mend` pass on a crate that forwards at least one diagnostic, with stderr captured through a pseudo-terminal, and show where the counter stops. If the cause is a different one, report it and fix that one.
- One owner writes to the progress line. A forwarded block goes through it like `write_status_notice` does: erase the line, print the block, redraw the spinner. Do not add a second writer of the terminal line.
- No hard-coded configuration value.

**Spec:**
1. In Full mode, after a forwarded diagnostic, the counter continues; at the end of the pass the line is cleared once.
2. **Tests, red on main first, each under one second:** change `forwarded_diagnostic_stops_progress_before_printing` to assert the new contract (the block prints, progress continues, a later counter update arrives). Name it in the commit message as an Assertion-Change.
3. Live proof: a captured run shows counter updates after a forwarded diagnostic; the director proves it in a cargo-tile mend cell.
4. CHANGELOG: one line.

**Files:**
- `crates/cargo-mend/src/compiler/build/progress.rs`
- `crates/cargo-mend/src/compiler/build/stderr.rs`
- `crates/cargo-mend/CHANGELOG.md` (phase 2h holds it; the director adds this line)

**Workers:** 1 writer.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean; live counter after a diagnostic in a cargo-tile mend cell.

### Phase 3 — cargo-port draws a frame only when something changed  · status: todo

**Depends on:** Phase 2b

#### Work Order

**Goal:** an idle cargo-port uses under 1% of one core and draws no frame when nothing visible changed. During builds it stays under 3% of one core and under 2 frames each second.

**Source:** showrunner, 2026-10-28; the user ranks CPU work above all other work. Measured on natedev at 11:12–11:25: the cargo-port main thread used 14% of one core; perf put 90% of it in `Path == Path` compares from `RootItem::at_path` (`crates/cargo-port/src/project/root_item.rs`), about 4 frames each second at about 30 ms each. Notes: `/tmp/claude-1000/-home-natepiano-rust-cargo-liner/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/scratchpad/cpu_investigation.md`, section "cargo-port: a full frame".

**Constraints:**
- From phase 2b: a `DiskUsage` reply can change a row from `Visible` to `Deleted` with the byte count still zero (`async_tasks/disk_handlers.rs` `apply_disk_usage` ≈65), so equal bytes do not mean equal visible state. Test that this change draws once and an identical later reply draws nothing.
- From phase 2d: the row error state for a manifest cargo rejects is a visible input. Test redraws for an unselected row's error, its recovery, and `ProjectDiscovered`.
- The detail cache owns `Option<DetailCacheKey>` (`tui/panes/data.rs` ≈21). Replace it with a named state that tells cleared details from details current for a key, like the new caches.
- Measure idle CPU only after the startup folder walk, watcher registration and reconciliation, and startup enrichment end (the walk alone uses several cores for tens of seconds in a debug build; scan completion, `dispatch.rs` ≈96, comes before the watcher's Ready, `watcher/runtime.rs` ≈427). Add a named log marker or a repeatable procedure for that point and start both 60 s windows after it.
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
4. **Tests, red first, each under one second:** an idle loop iteration with no change draws no frame; a background message that changes a row draws one; a `DiskUsage` reply for an unselected project with a new size updates its row, one that changes `Visible` to `Deleted` draws once, and an identical later reply draws nothing; the index finds the right project after insert, replace, regroup and rescan; a lookup by path returns the same project as the linear search for roots, members, worktrees and vendored packages; the disk cache does not recompute when nothing changed and does when a disk size changes.
5. **Live CPU measure:** CPU seconds over 60 s, idle (no input, no cargo running in the scratch roots), for the installed binary and the branch binary, with the method in `docs/cargo-port/as-built/overview.md`, after checking that no natedev build hold is in force. Record both numbers and the frames drawn per second from the frame counter. Then the same with a `cargo build` running in a scratch project and no spinner on screen.
6. CHANGELOG: one line in `crates/cargo-port/CHANGELOG.md`.

**Files:**
- `crates/cargo-port/src/tui/panes/data.rs` and `crates/cargo-port/src/tui/panes/system.rs` — the detail cache state
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
- From phase 3: retained `Deleted` rows and stored dismissals stay reachable through the path index, and each retention change keeps its caches current. The late-reply dismissal guard already exists with a test (`async_tasks/disk_handlers.rs` ≈147); the work is to keep it through a scan replacement. The tester's acceptance case covers a discovery folder removed with two projects below it, a completed rescan, a dismissal, and late replies.
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
