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

**As-built:** under `RecursiveRoot` (macOS) the registration plan in `crates/cargo-port/src/watcher/roots.rs` adds a recursive watch only for a configured root that no other configured root covers, in either root order (checkpoint 95138542). Each configured root stays a discovery boundary (`record_recursive_boundary`); coverage reads only paths that were watched (`recursively_watches`), so a failed parent registration still leaves the covered root watched. `PerFolder` is unchanged. The watcher resolves the cargo home once (`resolve_cargo_home` → `CargoHome::{Directory, Unavailable}`) and passes it to registration, so the registration tests use a test home and do not read the developer's `~/.cargo`. The one-watch-per-path invariant test in `runtime.rs` runs for both backends.

### Phase 2h — mend keeps a re-export another package uses  · status: done

**As-built:** the subtree re-export planner reads every mount of a physical source that mend can see by syntax, across all packages: `mod` items with or without `#[path]` (also inside function bodies) and `include!` with any path prefix and a literal path (`crates/cargo-mend/src/rust_syntax/module_map.rs`). A shared re-export changes only when every package that uses it gets the same edit (`fixes/subtree_reexport/scan.rs`); otherwise mend keeps it and reports one skipped manual rewrite. A mount mend cannot place (`UnobservedPackageReach::Unresolved`, a non-literal include) gives that package unknown reach, which keeps the re-export. Nested packages under a root package path are told apart by source membership, not a directory prefix. `resolve_visibility_proposals` holds its agreement state in an enum; tui_pane `attract::controller` names are `pub(super)` (checkpoint 44ecc942). Gotchas: a `mod x;` that a fix leaves empty stays; renamed custom macros and attribute or derive macros are outside the mount guarantee; raw identifier spellings are the next mend phase's work. Ruled out: deleting module declarations a fix leaves empty.

### Phase 2i — mend never writes the same import twice  · status: done

**As-built:** every fixer that adds or rewrites a use item (PreferModuleImport, ImportsAtTop, SubtreeReexport caller rewrites, facade redirects) hands its imports to one owner, `ModuleImports` (`crates/cargo-mend/src/fixes/imports/module_imports.rs`, conditions in `import_condition.rs`). It resolves each binding from module paths seeded by Cargo target roots (`rust_syntax::ModuleMap` via `ParsedSources::for_selection`), so equal paths to one module are one binding; identity is scope + target + name, and a public or cfg-gated re-export is kept while a private import is limited to the configurations it does not cover. A file with several module paths stays unresolved and its dependent rewrites are declined. A declined import edit goes back into `subtree_reexport::scan_selection`'s fact-exclusion loop, so the whole re-export change is re-planned without that finding (re-export kept, one skipped report, independent fixes applied). Raw identifier mounts (`r#include!`, `#[r#path]`, `#[r#cfg_attr]`) count; `CrateFiles.modules` keeps the `FileModulePath` enum (checkpoint e7f314d2). Gotchas: the per-fixer `import_plan` dedup is gone; any new fixer that writes imports must go through `ModuleImports`. Ruled out: filtering finished output to drop duplicates.

### Phase 2j — a mend cell keeps its progress bar after a diagnostic  · status: done

**As-built:** a forwarded compiler diagnostic goes through the one owner of the progress line in `crates/cargo-mend/src/compiler/build/progress.rs` (`write_forwarded_output`): erase the line, print the block, add a newline only when the block lacks one, redraw (checkpoint 32316514). The counter runs to the end of the pass, and an unterminated last diagnostic survives the redraw and the cleanup. Live proof: a real cargo-tile mend cell read 85% after the forwarded warning.

### Phase 2k — workspace lint passes on the cargo-berth reanchoring test  · status: done

**As-built:** the last checks of `side_branch_creation_during_stopped_apply_rebase_preserves_checkpoint_capture` (`crates/cargo-berth/tests/support/reanchoring.rs`) moved into `assert_reanchored_checkpoint_is_outstanding`, with every assert and step in the same order and no `allow` or `expect`, so workspace clippy's `too_many_lines` passes again after mend's autofix 46570b43 (checkpoint 806cbc5c). Lifecycle tests 145/145 pass.

### Phase 2l — mend runs in a project that pins a different Rust release  · status: done

**As-built:** `cargo mend` and `cargo mend --fix` compile every driver command with the rustc of the toolchain that built mend, so a project whose `rust-toolchain.toml` pins another Rust release no longer fails with E0514 (checkpoint d0b9c24e). `DriverCompiler` (`crates/cargo-mend/src/compiler/build/driver_compiler.rs`) is the one owner: `for_build()` resolves `<MEND_BUILD_SYSROOT>/bin/rustc` once per run, and `configure(command, wrapper)` sets `RUSTC` and `RUSTC_WORKSPACE_WRAPPER` together and removes the passthrough wrapper. `run_cargo_check` (`execute.rs`) calls it for the check and the fix validation build; no call site sets these variables by hand. A missing rustc in the build sysroot stops mend with an error that names the folder. When the compiler versions differ, Cargo keeps a second set of dependency artifacts in the same target folder; the README and CHANGELOG name this cost.

**Files:** `crates/cargo-mend/src/compiler/build/driver_compiler.rs`, `execute.rs`, `crates/cargo-mend/src/compiler/constants.rs` (`RUSTC_ENV`), `crates/cargo-mend/tests/cli_smoke.rs` (`fix_uses_build_compiler_with_external_path_dependency_and_unusable_project_rustc`), `crates/cargo-mend/README.md`, `crates/cargo-mend/CHANGELOG.md`.

**Gotchas:** a cli or diagnostics test that runs real cargo builds has a floor near 2 s; such a test runs mend once.

**Ruled out:** reinstalling mend on the project's release; changes to Hana, `verify.sh` or a project's toolchain file.

### Phase 2n — mend --fix keeps the imports of a module mounted with #[path]  · status: done

**As-built:** `cargo mend --fix` keeps a module import in a file mounted with `#[path = "…"]`, and the crate builds with nothing rolled back (checkpoint 73a51de7). `support::leaf_is_module` (`crates/cargo-mend/src/fixes/prefer_module_import/support.rs`) decides whether an import leaf names a module through `rust_syntax::ModuleMap::module_ancestors`, which follows `#[path]` mounts and the modules they declare; it no longer walks physical folders. `ModuleMap::declared_module_files` prefers the existing sibling file by default. The fixture `fix_imports_keep_modules_declared_in_path_mounted_test_files` (`tests/diagnostics/import_fixes.rs`) holds both Hana shapes, and `path_mounted_import_resolution_uses_declared_module_paths` pins the module path of a `#[path]` file and of a module it declares.

**Files:** `crates/cargo-mend/src/fixes/prefer_module_import/support.rs`, `crates/cargo-mend/src/rust_syntax/module_map.rs`, `crates/cargo-mend/tests/diagnostics/import_fixes.rs`, `crates/cargo-mend/CHANGELOG.md`.

**Gotchas:** module identity comes from `ModuleMap`, never from folder layout.

**Ruled out:** filtering fixes after planning.

### Phase 2o — mend --fix keeps the leading :: of a path it shortens  · status: done

**As-built:** `cargo mend --fix` replaces the whole absolute path, its leading `::` included, when it shortens a path such as `::bevy::picking::events::PointerCancel` to an imported name; the crate builds and nothing rolls back (checkpoint 8b5e6d59). `InlinePathVisitor` (`crates/cargo-mend/src/fixes/inline_path_qualified_type/visitor.rs`) starts the replacement span at `path.span()`, so the `::` is part of it, and ends it at the last identifier, so generic arguments stay. `full_path` keeps the leading `::`, and `resolve_occurrence` (`process.rs`) then resolves the inserted `use` as `::<path>`, so a local import with the same first name cannot capture it. The finding highlight length comes from the replaced byte span. The fixture `absolute_inline_paths_with_glob_prelude_build_after_fix` (`tests/diagnostics/inline_path_fixes.rs`) holds the Hana shape; `absolute_path_replacements_remove_the_anchor_and_keep_generic_arguments` and `absolute_paths_keep_external_identity_and_complete_highlight` pin the span, the anchor and the highlight.

**Files:** `crates/cargo-mend/src/fixes/inline_path_qualified_type/{visitor.rs,process.rs,scan.rs}`, `crates/cargo-mend/tests/diagnostics/inline_path_fixes.rs`, `crates/cargo-mend/CHANGELOG.md`.

**Gotchas:** an absolute path names an external crate whatever the local imports are; never absolutize it through scope imports.

**Ruled out:** filtering fixes after planning.

### Phase 2p — one mend run uses one wrapper, also across a reinstall  · status: done

**As-built:** one `cargo mend` run uses one rustc wrapper from start to end, so a reinstall of cargo-mend during a run has no effect on it (checkpoint 11eb5b33). `run()` (`crates/cargo-mend/src/main.rs`) opens `RunningExecutable` as its first action, before cargo metadata: `/proc/self/exe` on Linux, `env::current_exe()` elsewhere. The open handle keeps the old inode after an install replaces the path. `RunWrapper::resolve` (`crates/cargo-mend/src/compiler/build/execute.rs`) reads the revision from that handle and reuses a valid independent copy, or copies one from the handle (FICLONE on Linux, else a byte copy). `MendRunner` carries the one `RunWrapper` by value through analysis, cache recovery and fix validation. `wrapper_alias` returns `Result`; a wrapper that cannot be made is an error that names the cause, and `RunWrapper::configure` refuses a missing copy, so cargo never gets a path that does not exist. `run_wrapper_resolves_open_executable_after_source_replacement_and_removal` and `run_wrapper_stays_the_same_after_source_replacement_and_removal` pin the reinstall case without running cargo.

**Files:** `crates/cargo-mend/src/compiler/build/{execute.rs,mod.rs}`, `crates/cargo-mend/src/compiler/mod.rs`, `crates/cargo-mend/src/main.rs`, `crates/cargo-mend/src/fixes/runner/{mend_runner.rs,execute.rs}`, `crates/cargo-mend/CHANGELOG.md`.

**Gotchas:** the lint assertion guard reads `Assertion-Change:` lines from the phase work-order file and the commit message, not from the plan; an `Option` to `Result` signature change reads to it as a removed assertion. Open follow-ups F004 (copy-failure test repeats the staging path rule) and F005 (help and build-info need read access to the executable) are in `docs/cargo-port/cargo-liner-port-next.md`.

**Ruled out:** a global wrapper path; falling back to the install path when the copy fails; copying from the install path after cargo metadata.

### Phase 2r — a build-script probe through the mend wrapper reads its input  · status: done

**As-built:** on Linux, a build script under mend starts stable rustc (checkpoint 90886cda). rustup puts the pinned toolchain's lib folder on `LD_LIBRARY_PATH`, and cargo puts the selected compiler's rustlib folder there. Both come before rustc's RUNPATH, and neither holds the matching libLLVM, so rustc died at load before it read stdin (the rustix broken pipe, the libc libLLVM error and the autocfg "could not execute rustc"). `DriverCompiler` (`crates/cargo-mend/src/compiler/build/driver_compiler.rs`) holds `runtime_libraries` (`sysroot/lib`) on Linux. `DriverCompiler::configure` puts that folder first on the cargo command's `LD_LIBRARY_PATH` and keeps the inherited entries after it (from the command's env, else the process env). Only that Linux step can fail; on macOS `configure` returns `()`. `RunWrapper` (`execute.rs`) calls `DriverCompiler::for_build()?.configure(command, &self.path)`. `build_script_starts_compiler_with_its_runtime_libraries`, `build_script_compiler_probe_reads_stdin` (a probe larger than a pipe buffer, `PROBE_PAYLOAD_SIZE`) and `compiler_library_path_preserves_command_paths` pin it.

**Files:** `crates/cargo-mend/src/compiler/build/{driver_compiler.rs,execute.rs}`, `crates/cargo-mend/CHANGELOG.md`.

**Gotchas:** a step that can fail only on Linux makes Mac clippy flag `unnecessary_wraps` on a shared function; keep the fallible part in a Linux-only function.

**Ruled out:** a stdin change in the wrapper: the wrapper already reads stdin, and rustc failed at load.

### Phase 2q — tui_pane owns the expand/collapse model for cargo-port and cargo-handler  · status: done

#### As-built

`tui_pane::expansion` holds one expand/collapse model for tree lists. `ExpansionModel<K>` stores one override map (`K` to `ExpansionState::{Open, Closed}`) plus a default rule `fn(usize) -> bool` by depth; a row is open when its override says so, else when its default says so. Operations: `expand`, `collapse`, `toggle`, `expand_all`, `collapse_all`, `reset`, `retain`, `from_open_keys`/`open_keys`, and `collapse_row`, which on a closed or leaf row selects the parent and collapses it. Mutations return `ExpansionChange::{Changed, Unchanged}`. The app supplies its rows through the `ExpansionTree` trait (`RowExpansion` with key and depth, `RowParent`); cargo-port's row kinds stay in cargo-port. The expansion actions and their default keys (`EXPANSION_VIM_EXTRAS`) live in `tui_pane`. cargo-port's project list keeps its state in `ProjectListState.expanded: ExpansionModel<ExpandKey>`, closed at every depth, and behaves as before.

**Files:**
- `crates/tui_pane/src/expansion.rs` — the model, the tree trait, the result types and their unit tests
- `crates/tui_pane/src/keymap/expansion_action.rs`, `keymap/constants.rs` — shared actions and bindings
- `crates/cargo-port/src/tui/project_list/{list.rs,selection.rs,visible_rows.rs,expand_state.rs}` — the project list through the model
- `crates/cargo-port/src/tui/app/navigation/expand.rs`, `tui/keymap/`, `tui/project_list_state.rs` — callers and bindings

**Binds later work:** cargo-handler's parent/child rows use `ExpansionModel` with depth 0 open (handler-unit). After a roots change, `ProjectList::replace_roots_from` calls `expanded.retain` with the live expandable keys.

**Gotchas:** a test that compares expansion through `open_keys()` changes assertion shape and needs an `Assertion-Change:` line.

### Phase 2s — mend --fix keeps the cfg of an import it moves  · status: done

#### As-built

When `cargo mend --fix` moves an in-body `use` to module scope (`fixes/imports_at_top`), the moved import keeps every cfg of the items around it. The shared tracker `gate_tracking_visit!` (`fixes/imports/gate_tracking.rs`) collects cfg attributes from items, fn parameters (`PatType`), fields, generics, patterns and the other attribute-bearing expression nodes; `inline_path_qualified_type` uses the same tracker. This fixes the eight `use libproc::…` imports that lost `#[cfg(target_os = "macos")]` in cargo-handler and broke the Linux build.

**Files:**
- `crates/cargo-mend/src/fixes/imports_at_top/in_body_use_finder.rs` — finds in-body imports with their gates
- `crates/cargo-mend/src/fixes/imports/gate_tracking.rs`, `fixes/imports/mod.rs` — the shared cfg tracker
- `crates/cargo-mend/src/fixes/imports_at_top/gate_tests.rs` — one regression test per node kind, applied through `imports::apply_fixes`

**Gotchas:** a new cfg-bearing syn node needs a case in `gate_tracking_visit!`, or the imports under it lose their cfg when they move.

### Phase 2t — mend --fix never writes a path into a struct pattern or struct literal field  · status: done

#### Work Order

**Goal:** `verify.sh lint hana` passes with cargo-mend 0.23.0-dev on the hana tree that 0.22.3 passes; no mend rewrite gives code that does not compile.

**Source:** showrunner, 2026-11-03, from hana: cargo-mend 0.23.0-dev broke `verify.sh lint hana` on `init/catalyst` ad937ff7ec plus demo Phase 28. Its rewrites gave 13 compile errors in 4 test files (`crates/hana/tests/frame_time_cables.rs`, `cable_motion/opened_routes.rs`, `cable_motion/patch_stubs.rs`, `screen_assembly/mod.rs`), then mend rolled back. One error is "expected one of `,`, `:`, or `}`, found `::`": a path written where a struct pattern or struct literal expects a field name. The same tree passes with mend 0.22.3. hana's log is gone.

**Constraints:**
- The tree: `~/rust/hana` at ad937ff7ec has three of the four files (read them with `git -C ~/rust/hana show ad937ff7ec:<path>`). The demo Phase 28 worktree `/home/natepiano/rust/tool-based-ui-demo` belongs to another session: read it only, never edit, build or lint in it.
- Suspects: the fixes changed since `cargo-mend-v0.22.3` (`git log cargo-mend-v0.22.3..HEAD -- crates/cargo-mend`), first `imports_at_top` (phase 2s gate tracking, phase 2i, 2n), `inline_path_qualified_type`, and the leading `::` keep of phase 2o. Find the exact rewrite from the code and the four files before any change.
- From phase 2s: a new syn node in a fix needs a case in `gate_tracking_visit!` (`fixes/imports/gate_tracking.rs`).
- A rewrite must never replace a field name, a field-init shorthand (`Foo { x }`) or a field pattern shorthand (`Foo { x, .. }`) with a path. Where a shorthand binds a name the fix shortens or qualifies, write `x: <path>` or leave it unchanged.

**Spec:**
1. **Red first:** the smallest fixture that gives the parse error (or one of the 13 errors) through the real fix path (`imports::apply_fixes` or the fix's own apply entry), with the parsed output checked by `syn::parse_file`. One test for each distinct cause the four files show.
2. Fix the cause in the fix that writes it.
3. Every mend fix's output in the new tests parses and keeps the program's meaning.
4. CHANGELOG: one line in `crates/cargo-mend/CHANGELOG.md`.

**Files:**
- `crates/cargo-mend/src/fixes/` — the fix that writes the path, and its tests
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer owns the code and the tests.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean; no test takes a second; the red tests fail on the pre-fix tree and pass after; the summary names each of the 13 errors' cause or says which cause it shares.
- Director gate (showrunner, 2026-11-03): the new mend binary runs `mend --all-targets --workspace --fix` on a copy of hana ad937ff7ec plus the demo Phase 28 changes, with no rollback, and `cargo check --workspace --all-targets` on the fixed copy gives 0 errors.

### Phase 2u — mend --fix never adds an import that a glob already brings  · status: todo

#### Work Order

**Goal:** `mend --fix` on the hana copy from Phase 2t adds no import that rustc reports as unused, so `verify.sh lint hana`, which denies warnings, passes.

**Source:** showrunner, 2026-11-03, from the Phase 2t director gate: on the fixed hana copy, `cargo check` gives one new warning, `unused import: bevy::ecs::system::RunSystemOnce`. `mend --fix` added `use bevy::ecs::system::RunSystemOnce as _;` to `crates/hana/tests/arrange_glyph/envelope.rs`, which already has `use super::*;`. Hana stays blocked until this phase lands.

**Constraints:**
- The fixed and clean hana copies are in the director's scratchpad; the director gives their paths. Read `/home/natepiano/rust/tool-based-ui-demo` only; never edit, build or lint in it.
- Find the fix that writes the import from the code and the hana file before any change.
- An import that a fix adds must not duplicate a name or a trait (`as _`) that a glob import in the same module already brings.

**Spec:**
1. **Red first:** the smallest fixture through the real fix path that adds the duplicate import, with a check that the output has no unused import.
2. Fix the cause in the fix that writes the import.
3. CHANGELOG: one line in `crates/cargo-mend/CHANGELOG.md`.

**Files:**
- `crates/cargo-mend/src/fixes/` — the fix that adds the import, and its tests
- `crates/cargo-mend/CHANGELOG.md`

**Workers:** 1 writer owns the code and the tests.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean; no test takes a second; the red test fails on the pre-fix tree and passes after. Director gate: the new mend binary runs `--fix` on a clean copy of the Phase 2t hana tree with no rollback, and `cargo check --workspace --all-targets` on the fixed copy gives 0 errors and 0 warnings.

### Phase 3 — cargo-port draws a frame only when something changed  · status: done

**Depends on:** Phase 2b

#### As-built

The event loop (`tui/terminal/event_loop.rs`) draws a frame only when the frame key or the drawn screen changed since the last frame; the 1 s idle heartbeat draws nothing when nothing changed. A perf-log line (`TUI_PANE_LOG=tui_pane::perf=trace`, `drawn_frame_count`, `drawn_frames=N`) reports the drawn frames each second. `ProjectList` keeps a path index from project path to its place in the roots, rebuilt when the roots change, so `at_path`, its siblings, `visible_row_disk_usage` and the per-row render lookup do no linear search. List rows, disk totals, column widths and details recompute only when their typed keys change (`NotComputed` / `Current(key)`); the detail cache stores `DetailCacheState` (`Cleared` / `Current(key)`). `compute_disk_cache` clones no vector per child node. Measured on release builds, one scratch project: idle 0.267/0.250/0.250% of one core (main 0.217/0.183/0.217%); during a build in the worktree 0.200/0.664/0.348% (main 0.646/0.201/0.439%) at 1.00/1.05/1.33 frames each second.

**Files:**
- `crates/cargo-port/src/tui/terminal/event_loop.rs`, `event_loop_tests.rs` — draw on change, frame counter, fixed-time test source
- `crates/cargo-port/src/tui/app/navigation/cache.rs` — typed cache keys
- `crates/cargo-port/src/tui/project_list/list.rs`, `path_index_tests.rs` — path index and its upkeep tests
- `crates/cargo-port/src/tui/panes/project_list/disk.rs` — disk cache without per-node clones
- `crates/cargo-port/src/tui/panes/data.rs` — `DetailCacheState` storage

**Gotchas:** each CPU pane sample changes the screen, so an idle cargo-port draws about 1.15 frames each second. ratatui-core 0.1.2 keeps `Frame::cursor_position` crate-private and has no public cursor visibility. The per-frame candidate render left idle CPU above main's; Phase 3b removes it.

### Phase 3b — A drawn frame renders once, into the real terminal  · status: todo

**Depends on:** Phase 3

#### Work Order

**Goal:** idle CPU on the branch binary is below main's, beyond the spread over 3 pairs.

**Source:** showrunner, 2026-11-03, holding phase 3 (ef8fce7c) before its merge. Release idle, 3 pairs, one scratch project: main 0.217/0.183/0.217% of one core, branch 0.267/0.250/0.250%; the gap is larger than each side's spread. Both draw about 1.15 frames each second at idle, because each CPU pane sample changes the screen.

**Cause (from the code):** `draw_frame` (`tui/terminal/event_loop.rs` ≈275) makes a new `Terminal<TestBackend>` for each frame, renders the whole UI into it, clones its buffer, compares it with the last frame, then copies it into the real terminal. Main renders once into the real terminal. ratatui's `Terminal::draw` already writes only the cells that changed.

**Constraints:**
- Draw with the public `Terminal::draw` when the frame key says a draw is due; remove the per-frame candidate `Terminal<TestBackend>`. Do not expand the frame key.
- Classify each frame as `Drawn` or `Unchanged` by its completed buffer compared with one retained screen. Never call `get_cursor_position`: with `CrosstermBackend` it queries the terminal and waits for a reply, which adds a round trip to each frame and exits cargo-port in a terminal that does not reply. cargo-port sets no cursor, so the buffer is the whole screen. ratatui-core 0.1.2 keeps `Frame::cursor_position` crate-private; do not patch ratatui.
- Keep the frame key, the draw-on-change tests, every existing assertion and the F015 fixed-time test source. The frame counter counts only `Drawn` frames.
- No new allocation for each frame in the draw path.

**Spec:**
1. One render for each drawn frame, into the real terminal.
2. **Tests, red first, each under one second:** a drawn frame calls the UI render once; an unchanged frame key renders nothing; the existing event loop tests stay green.
3. **Live CPU measure** (unit director): 3 idle pairs, main against the branch, release builds, with the method in `docs/cargo-port/as-built/overview.md`.

**Files:**
- `crates/cargo-port/src/tui/terminal/event_loop.rs` and `crates/cargo-port/src/tui/terminal/event_loop_tests.rs`
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer (impl4) owns the code and the tests; the unit director owns the live measure.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; branch idle CPU below main's beyond the spread over 3 pairs.

### Phase 3c — The detail cache API takes a named state  · status: todo

**Depends on:** Phase 3

#### Work Order

**Goal:** no caller of the detail cache reads or passes a bare absence; cleared details and details current for a key are two named states.

**Source:** phase 3 review. Phase 3's Constraints required the detail cache APIs and their tests to take and return the named state. Phase 3 changed the storage to `DetailCacheState` (`tui/panes/data.rs` ≈68, `Cleared` / `Current(key)`), but `detail_is_current`, `set_detail_stamp` and `Panes::clear_detail_data` still take `Option<DetailCacheKey>`, and `impl From<Option<DetailCacheKey>> for DetailCacheState` converts at the boundary.

**Constraints:**
- The desired state comes from the selection (`tui/app/navigation/cache.rs` ≈80–109). No selection is its own named outcome, not `None`; convert once where the selection is read.
- Remove `impl From<Option<DetailCacheKey>> for DetailCacheState` when no caller needs it.
- Behavior stays the same; this phase changes types only.

**Spec:**
1. `detail_is_current`, `set_detail_stamp` and `Panes::clear_detail_data` take the named state. `repo_handlers.rs` ≈650 and `navigation/cache.rs` pass a named state.
2. **Tests, each under one second:** the existing detail cache tests in `tui/panes/system.rs` use the named states; add one for no selection and one for cleared details of a selected row.

**Files:**
- `crates/cargo-port/src/tui/panes/data.rs` and `crates/cargo-port/src/tui/panes/system.rs`
- `crates/cargo-port/src/tui/app/navigation/cache.rs`
- `crates/cargo-port/src/tui/app/async_tasks/repo_handlers.rs`

**Workers:** 1 writer owns the code and the tests.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; no `Option<DetailCacheKey>` in a detail cache API.

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
- From phase 2q: `ProjectList::apply_expanded` (`tui/project_list/list.rs`) skips expansion targets that are not in the installed tree. Merge the retained Deleted rows into the new roots first, then prune expansion keys and restore the saved expansion targets. Test it with reordered roots and a selected Deleted member under an open group: the row stays visible, the selection stays on it, and the group stays open.
- From phase 3: the indexed path lookup in `ProjectList` (`tui/project_list/list.rs` ≈446) returns one `None` for an unknown path and for a stored index route that no longer points at a project. Return a named result (an enum) so the two outcomes differ, and make each caller handle both. Add one test for each outcome in `tui/project_list/path_index_tests.rs`. This phase changes the retained rows in that index, so the stale-route outcome is reachable here.
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
- `crates/cargo-port/src/tui/app/async_tasks/tree.rs` — rescan keeps Deleted entries (entry point `prepare_rescan`)
- `crates/cargo-port/src/tui/app/async_tasks/dispatch.rs` — the completed scan result keeps Deleted entries
- `crates/cargo-port/src/tui/app/async_tasks/disk_handlers.rs` — late replies after dismiss
- `crates/cargo-port/src/tui/app/async_tasks/deleted_rescan_tests.rs` — new; tester
- `crates/cargo-port/src/tui/terminal/no_color_tests.rs` — new; tester
- `crates/cargo-port/src/tui/project_list/` — the Deleted entry state, if the root type holds it; the named indexed lookup result and its tests in `path_index_tests.rs`
- `crates/cargo-port/src/tui/terminal/` — NO_COLOR attribute output; not event_loop.rs or event_loop_tests.rs, which Phase 3b owns
- `crates/cargo-port/src/tui/sccache/render.rs` — popup width
- `crates/cargo-port/src/tui/panes/` — the Git pane value fit, once the cause is found; not data.rs or system.rs, which Phase 3c owns
- `crates/cargo-port/CHANGELOG.md` — add the lines after Phase 3b's checkpoint

**Workers:** 1 writer + 1 tester.
- `impl` — specs 1–4 and 7; hub: `crates/cargo-port/src/tui/app/async_tasks/tree.rs`. Not the tester's files.
- `test` — spec 5, in `tui/app/async_tasks/deleted_rescan_tests.rs` and `tui/terminal/no_color_tests.rs`, plus the sccache and Git pane cases in test modules beside those files, each named in the tester's first board post; `impl` registers them.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; the live check and the four shots of spec 6, with a design check pass. The checkpoint adds a Change-Review line for each visible fix.

### Phase 5 — A CPU sample costs only the CPU pane  · status: todo

**Depends on:** Phase 3b (both edit `tui/terminal/event_loop.rs`)

#### Work Order

**Goal:** a CPU pane sample that changes nothing else does not render every pane. Idle CPU on the branch binary is not above main's.

**Source:** phase 3 review (event_loop.rs ≈285): on a CPU-only wake, the frame check allocates a backend, renders the whole UI to a candidate buffer and clones it. Phase 3's release idle measure (3 pairs, one scratch project): main 0.217/0.183/0.217% of one core, branch 0.267/0.250/0.250%. The median change (+0.033 points) is inside main's spread (0.034), so the cause is not proven. Astra's advice: `/tmp/claude/astra/20261102T204624Z-1292297/review.md`.

**Constraints:**
- Phase 3b removes the per-frame candidate render, allocation and copy that the source above names. Measure again after Phase 3b: 3 idle pairs, main's release build against the branch's, and one CPU-only wake on the branch (render calls, time).
- Build the partial render only when those 3 pairs show branch idle CPU above main's beyond the spread. Otherwise the phase completes with no code change: record the pairs in its As-built and close it.
- Full and CPU-only renders share one render setup in `tui/render.rs` (`render_tiled_panes` ≈144 and its render inputs). The event loop does not copy the render-context or pane-chrome rules.
- Phase 3 already makes an identical CPU sample return false in `CpuPane::tick` (`tui/panes/cpu/pane.rs` ≈122).
- A partial render applies only to an isolated CPU change with stable geometry. Do a full render for the first frame, a layout change (the core count changes the layout, `render.rs` ≈169), any other pending change, and an active overlay or toast.
- For a partial render, keep the last frame's screen, clear the CPU rect, and render it through `render_panes` with a CPU-only layout; the framework draws borders and dividers (`tui_pane` `framework/render.rs` ≈122). Keep the other cells, the cursor and the equality check.

**Spec:**
1. A CPU-only wake renders the CPU pane only, or draws nothing when its visible rows did not change.
2. **Tests, red first, each under one second,** through the production frame path with CPU samples on a channel and a fixed time, counting pane render calls: a visible CPU change renders no other pane and draws once; a change in a hidden core row draws nothing; the buffer and cursor equal a full render; a layout change and a popup over the CPU pane each do a full render.
3. **Live CPU measure:** 3 idle pairs, main against the branch, with the method in `docs/cargo-port/as-built/overview.md`.

**Files:**
- `crates/cargo-port/src/tui/terminal/event_loop.rs` and `event_loop_tests.rs` — the CPU-only frame path and its tests
- `crates/cargo-port/src/tui/panes/cpu/` — the CPU pane render, if needed
- `crates/cargo-port/src/tui/render.rs` — the render setup shared by full and CPU-only renders
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer (impl4) owns the code and the tests; the unit director owns the live measure.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second; branch idle CPU not above main's beyond the spread over 3 pairs.
