# port


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
- **Invariants:** no test takes a second; a deleted project stays in the list, struck through, until the user dismisses it or rescans (user, 2026-11-05).

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

#### As-built

`mend --fix` no longer breaks code when `prefer_module_import` rewrites a path. `PreservedImports` keeps each binding whose uses cannot take a qualified path: an imported macro binding, a field name in a macro call, a field-init shorthand or field pattern shorthand, and a binding that an external child module uses. The import and all its uses stay together. `ReferenceCollector` uses its lexical scope information for field shorthand. A shared descendant import guard protects named imports and glob imports across child modules.

**Files:**
- `crates/cargo-mend/src/fixes/prefer_module_import/scan.rs` — the preserved imports and one test for each cause
- `crates/cargo-mend/CHANGELOG.md`

**Gotchas:** the macro token walker cannot tell a field name from a function name, so a binding used inside a macro call stays imported.

### Phase 2u — mend --fix never adds an import that a glob already brings  · status: done

#### As-built

`mend --fix` keeps a local import when a glob in the destination scope can supply the same binding or trait, so the fix adds no import that rustc reports as unused. `lint hana`, which denies warnings, passes on the copy that gave the warning.

**Files:**
- `crates/cargo-mend/src/fixes/imports_at_top/in_body_use_finder.rs` — finds the glob in the destination scope
- `crates/cargo-mend/src/fixes/imports/module_imports.rs` — the shared check for a binding that a glob brings
- `crates/cargo-mend/CHANGELOG.md`

### Phase 2v — widget gets the hana construct that crashes rustc  · status: done

#### As-built

No code. On hana `39e5c19033`, rustc 1.99.0 stops with `assertion failed: !deep_decl.is_glob_import()` (rustc_resolve imports.rs:583) when it builds test code; mend hits it only because it builds `--all-targets`. The construct: `use super::*;` in the cfg(test) module `rear_patch_panel::tests` (`crates/hana/src/tool/rear_patch_panel/mod.rs`), which brings a helper that the parent module also re-exports. Widget replaced the glob with explicit imports in `0c4e265f32`. Neither widget's worker nor this unit made a small sample that reproduces the panic; Codex refused the reduction task three times (content filter). The rustc issue can cite the commit pair `39e5c19033` / `0c4e265f32`.

### Phase 2w — mend --fix never makes a function too long for clippy  · status: done

#### As-built

`mend --fix` measures each function that a fix changes. It formats the new file text with the workspace rustfmt settings and counts body lines as clippy counts them: no blank lines and no comment lines. The limit comes from the workspace `clippy.toml` key `too-many-lines-threshold`, else the clippy default. A function that a fix moves from within the limit to over the limit keeps all its names and its function-scope imports; other functions in the file keep their rewrites. A function already over the limit, or with `allow` or `expect` on `clippy::too_many_lines`, keeps its earlier behavior. A kept function gives no warning and no failed exit. A notice names the file, the function and the count, and says to split the function; a second `--fix` run changes nothing and gives the same notice. mend never splits a function.

**Files:**
- `crates/cargo-mend/src/fixes/runner/function_length/` — the plan, the formatter, the function collector, the limits and the fixture
- `crates/cargo-mend/src/fixes/runner/apply.rs`, `combine.rs`, `execute.rs`, `mend_runner.rs`, `mod.rs`, `plan.rs` — the fix path holds the plan
- `crates/cargo-mend/src/fixes/imports/import_scan.rs`, `src/rust_syntax/lexical_regions.rs` — scans and the line count
- `crates/cargo-mend/src/reporting/outcome.rs` — the notices
- `crates/cargo-mend/CHANGELOG.md`

**Gotchas:** `Formatter::detect` starts up to two short processes per mend scan. A relocation pass that depends on a kept function stays out as a whole, which keeps more relocations than the one caller needs.

### Phase 2x — mend --fix keeps an import that a child module reaches through super  · status: done

#### As-built

`mend --fix` keeps a parent import that a child module reaches through a qualified path, such as `super::contrast_failures()` in an inline test module, a nested module, a file child or a macro argument. The descendant check (`DescendantAccessCollector` in `prefer_module_import/descendant_imports.rs`) records each `use`, glob, qualified path and macro-token path in a child module as a `DescendantBindingAccess`; the candidate filter keeps every import that one of them reaches. syn keeps macro bodies as tokens, so `collect_macro_paths` walks `ident :: ident` sequences through token groups. Five tests in `scan.rs` compile the result with rustc and check that a second pass changes nothing. On the Phase 4b tree, the new binary applied 7 import fixes with no rollback.

**Files:**
- `crates/cargo-mend/src/fixes/prefer_module_import/descendant_imports.rs`, `scan.rs` — the collector and its tests
- `crates/cargo-mend/CHANGELOG.md`

**Gotchas:** the tests run `scan_file`, `apply_fixes` and rustc, not the full `--fix` path, so no test takes a second; the `--fix` path and its rollback were proved by the director's lint gate only.

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

### Phase 3b — A drawn frame renders once, into the real terminal  · status: done

**Depends on:** Phase 3

#### As-built

`draw_frame` (`tui/terminal/event_loop.rs`) renders each drawn frame once, into the real terminal, with the public `Terminal::draw`. The per-frame `Terminal<TestBackend>` is gone. A frame is `Drawn` or `Unchanged` by a comparison of its completed buffer with one retained screen, and the frame counter counts only `Drawn` frames. The draw path adds no allocation for each frame. Release idle over 3 pairs: installed 0.200/0.217/0.217%, branch 0.200/0.217/0.217% of one core.

**Files:**
- `crates/cargo-port/src/tui/terminal/event_loop.rs`, `event_loop_tests.rs`
- `crates/cargo-port/CHANGELOG.md`

**Gotchas:** the draw path never calls `get_cursor_position`. With `CrosstermBackend` it queries the terminal and waits for a reply, which exits cargo-port in a terminal that does not reply. cargo-port sets no cursor, so the buffer is the whole screen.

### Phase 3c — The detail cache API takes a named state  · status: done

**Depends on:** Phase 3

#### As-built

`detail_is_current`, `set_detail_stamp` and `Panes::clear_detail_data` take `DetailCacheState` (`Cleared` / `Current(key)`). `tui/app/navigation/cache.rs` converts the selection once: a selected row gives `Current(key)` and no selection gives `Cleared`. `impl From<Option<DetailCacheKey>> for DetailCacheState` is gone. Behavior is unchanged.

**Files:**
- `crates/cargo-port/src/tui/panes/data.rs`, `panes/mod.rs`, `panes/system.rs` — the state and its API; tests for no selection and for cleared details of a selected row
- `crates/cargo-port/src/tui/app/navigation/cache.rs`, `app/async_tasks/repo_handlers.rs` — callers pass the named state

### Phase 3d — The discovery frame test takes no background reply  (merged 3fb98ed6e, checkpoint dd7c6a70c)

#### As-built

`project_discovered_draws_one_new_frame` discovers a non-Rust root, which starts no `cargo metadata` task, so no reply can join the same poll. The test keeps every assertion.

### Phase 4 — A Deleted row stays on the list and shows its mark  · status: done

**Depends on:** Phase 3

#### As-built

A project marked Deleted stays in its root, struck through and dismissable, also under `NO_COLOR=1`, until the user dismisses it or rescans (the user, 2026-11-05; Phase 4a removed the rescan retain that Phase 4 added). Late zero-byte and positive disk replies do not bring back a dismissed row. A rescan restores the saved choice (`last_selected`) when it is selectable, else the first visible row, and never falls back to the current path. Navigation, a focus click and a dismissal each record the choice.

`ProjectList::at_path` returns `ProjectPathLookup<T>` (`Found`, `UnknownPath`, `StaleRoute`), and each caller handles every outcome. Under `NO_COLOR`, the terminal output writes text attributes without colour codes and sends no colour reset that clears them. The sccache popup width is the widest row plus equal left and right padding. A Git pane value is cut at its end with an ellipsis, never from its middle; below inner width 11 a Git row shows only its label, in the full inner width.

**Files:**
- `crates/cargo-port/src/tui/project_list/list.rs`, `grouping.rs`, `visible_rows.rs`, `path_index_tests.rs` — the named lookup and its tests
- `crates/cargo-port/src/tui/app/async_tasks/tree.rs`, `dispatch.rs`, `disk_handlers.rs`, `deleted_rescan_tests.rs` — rescan, completed scan, late replies and their tests
- `crates/cargo-port/src/tui/app/navigation/selection.rs`, `app/selection_paths.rs`, `app/dismiss.rs`, `input/dispatch.rs`, `interaction.rs`, `finder/dispatch.rs` — record the choice
- `crates/cargo-port/src/tui/terminal/mod.rs`, `no_color_tests.rs` — attributes without colour
- `crates/cargo-port/src/tui/sccache/render.rs`, `popup_padding_tests.rs` — popup padding
- `crates/cargo-port/src/tui/panes/git/render.rs`, `value_fit_tests.rs` — Git value fit
- `crates/cargo-port/CHANGELOG.md`

**Gotchas:** the restored selection depends on four selection handlers outside the rescan code; a new way to choose a row must record the choice. Only a live run showed that a rescan replaced the live selection; the tests that pin it now live in `deleted_rescan_tests.rs`.

### Phase 4a — A rescan clears Deleted rows again  · status: done

**Depends on:** Phase 4

#### As-built

A deleted project stays on the list, struck through and dismissable, until the user dismisses it or rescans (the user, 2026-11-05: "they stay on the list till i either dismiss them or i rescan"). A full rescan clears the roots (`ProjectList::clear` from `prepare_rescan`) and installs only what the scan found. Individual project refreshes keep deletion states through `retain_deletion_states`, `preserve_replacement_deletion_states` and `preserve_submodule_deletion_states`. A saved choice (`last_selected`) that names a removed row stays saved; the first visible row is selected and is not saved as the choice.

**Files:**
- `crates/cargo-port/src/tui/project_list/list.rs`, `path_index_tests.rs` — the replacement without retained rows, and its tests
- `crates/cargo-port/src/tui/app/async_tasks/tree.rs`, `dispatch.rs`, `deleted_rescan_tests.rs` — the rescan, the completed scan and their tests
- `crates/cargo-port/src/tui/app/mod.rs` — the caller
- `crates/cargo-port/CHANGELOG.md`, `docs/cargo-port/as-built/overview.md`

### Phase 4b — cargo-port's selection, contrast and narrow panes read clearly  · status: done

#### As-built

- `theme_roles::paint_selection(frame, area, selection)` is the shared last selection pass. A selected row has one fill and one text colour across the inner row width, with a 4.5:1 floor checked by `covered_contrast` (`crates/tui_pane/src/bar/status_bar_contrast_support.rs`). Theme colours moved in `themes/builtins.rs` and `themes/*.toml`, which stay equal.
- Under `NO_COLOR`, an Active row is reversed, a Remembered row is bold, and a focused pane has a thick border. `prepare_terminal_buffer` removes every background, so CPU bars keep block glyphs. In colour, the bars draw with cell background.
- A Deleted row shows `[x]` in Disk and strikes only its name and value text. `PackageDeletion` (Present, Deleted), read from `project_list.is_deleted`, owns the Package pane's deleted title and Disk rows.
- A pane too narrow to show text is hidden through `PaneVisibility`. A hidden pane leaves the Tab order and has no mouse areas. `move_focus_from_hidden_pane` moves focus once, and focus stays in place when width returns. CI Runs draws only whole columns.
- `FIT_TEXT_ELLIPSIS` is `…`. `MIN_FITTED_TEXT_WIDTH` is the ellipsis width plus one, and the label-only transition width is its own value. The two Git rate rows read `Core` and `GraphQL`.
- The sccache popup shows the true scroll position, a highlight to the right border, and a reason row in plain words for each non-cacheable reason. Pane titles and rows use sentence case, and `Og`, `Mn` and `home/end ends` use common words. One target kind has one name, and `No lint runs` shows once.
- Languages rows separate parent rows from child subtotals (`parent_position` in `lang/render.rs`), and the total sums parents only. Projects has a right margin equal to its left margin.

**Files:**
- `crates/cargo-port/src/tui/render/mod.rs`, `tui/render/readability_tests.rs` — the frame, the narrow layout and their tests
- `crates/cargo-port/src/tui/test_support/mod.rs`, `tui/test_support/readability.rs` — shared drawn-buffer helpers
- `crates/cargo-port/src/tui/panes/lang/constants.rs` — Languages column constants
- `crates/cargo-port/src/tui/panes/*/readability_tests.rs`, `tui/sccache/readability_tests.rs`, `tui/terminal/no_color_tests.rs` — drawn-buffer tests for each surface
- `crates/cargo-port/src/themes/builtins.rs`, `crates/cargo-port/themes/*.toml` — selection and label colours
- `crates/tui_pane/src/pane/chrome.rs`, `pane/frame.rs`, `pane/state.rs` — the `NO_COLOR` focus border and selection

**Binds later work:** `theme_roles::paint_selection` runs last across the inner row width. `Option<TargetsData>` means absent content, not a Deleted project. `tui_pane::selection_style` (`pane/state.rs`) also serves cargo-tile and cargo-handler. `PackageDeletion` owns the Package pane's deleted title and Disk rows. `narrow_layout_keeps_git_values_and_hides_unreadable_panes` and `narrow_layout_moves_focus_once_and_restores_panes_without_moving_focus` pin the narrow layout.

**Gotchas:** a live capture in xterm can draw a symbol from a thin font, so the capture uses a fontconfig file that rejects Noto Sans Symbols 2. Heavy `┃` draws one dim pixel row at each cell boundary in a capture with all five fonts probed.

**Ruled out:** a separate selection paint in each pane; one shared last pass owns the selection.

### Phase 4d — Drawing code takes the colour mode as a value  · status: done

#### As-built

Drawing code does not read `NO_COLOR`. `tui_pane::ColorMode { Colored, NoColor }` is the value, and `ColorMode::from_environment()` is the only caller of `Colored::ansi_color_disabled_memoized()` in `crates/`. Each app reads the mode once at startup and holds it in `Framework::color_mode` (default `Colored`). The `tui_pane` suite passes with `NO_COLOR=1` and without it. The drawn output does not change.

Signatures: `default_pane_chrome(mode)`, `selection_style(focus, mode)`, `PaneSelectionState::overlay_style(self, mode)`, `patch(style, mode)`, `prepare_terminal_buffer(buffer, mode)`. `PaneChrome` holds the mode; `PaneChrome::block` and `GridLines::render` read it. The `cargo-berth` board has `pane_chrome(mode)`. `DescriptionBlock::render` is removed; `render_with_selection` remains.

Tests state their mode. `make_app` selects `ColorMode::Colored`. `make_app_with_color_mode(projects, mode)` passes the mode through `App::new`, so a child-process test proves the assignment in `app/construct.rs`.

**Files:**
- `crates/tui_pane/src/color_mode.rs` — the value and the only environment read
- `crates/tui_pane/src/pane/chrome.rs`, `frame.rs`, `state.rs` — drawing code takes the value
- `crates/cargo-port/src/tui/terminal/run.rs`, `tui/app/construct.rs` — cargo-port's app edge
- `crates/cargo-handler/src/app.rs`, `crates/cargo-tile/src/app.rs`, `crates/cargo-berth/src/board/tui.rs` — the other app edges
- `crates/cargo-port/src/tui/test_support/mod.rs` — test constructors with a stated mode

**Binds later work:** the colour-clearing step and the popup border take the mode from `Framework::color_mode`. No drawing code reads the environment.

**Gotchas:** `verify.sh test tui_pane` can reuse a `--local` `NO_COLOR=1` record, because the cache key has no `NO_COLOR`. Use `--no-cache` for a true run without it.

### Phase 4f — Bold and reverse survive NO_COLOR in every tui_pane app  · status: done

#### As-built

Under `NO_COLOR=1` in a real terminal, the focused title is bold and the selected row is reversed in `cargo-port`, `cargo-handler`, `cargo-tile` and the `cargo-berth` board. `tui_pane::ColorMode::clear_colors(self, &mut Buffer)` sets each cell's `fg` and `bg` to `Reset` under `NoColor`, and keeps text and modifiers. Under `Colored` it changes nothing.

`cargo-port`'s `prepare_terminal_buffer` calls the step and keeps its VS16 work. `cargo-handler` calls it on its full frame and on its status clock frame, before `painted_frame` caches the buffer. `cargo-tile` and the `cargo-berth` board call it at the end of `draw`. Crossterm keeps its own suppression for any write outside the buffer.

**Files:**
- `crates/tui_pane/src/color_mode.rs` — the step and the backend byte test `no_color_backend_keeps_bold_and_reverse_before_text`
- `crates/cargo-port/src/tui/render/mod.rs` — `prepare_terminal_buffer` calls the step
- `crates/cargo-handler/src/app.rs` — both draw exits call the step
- `crates/cargo-tile/src/app.rs`, `crates/cargo-berth/src/board/tui.rs` — `draw` calls the step

**Binds later work:** a new `tui_pane` app or draw exit calls `ColorMode::clear_colors` after it draws.

**Gotchas:** under `NO_COLOR`, crossterm writes `ESC[;m` for each colour change, and the backend sets modifiers first, so a cell with a colour loses bold and reverse. A tmux capture rewrites cell state: only a raw pty capture proves the bytes.

**Ruled out:** `force_color_output`.

### Phase 4c — cargo-port's remaining screen defects read clearly  · status: done

#### As-built

- Project list: every column stays at every width. The project name gives up its width first and ends with `…`. Projects takes at most the window less 30 cells, so Package and Git keep their width at 60 columns. At 80 columns Targets takes the place of CPU.
- Package and Git at 80 columns: a value that does not fit beside its label moves to the next row, labels stay whole, and a label stays on the page of its value. Package draws two columns only from 37 inner cells (`MIN_TWO_COLUMN_INNER_WIDTH` in `panes/package/render.rs`).
- `tui_pane`: `rule_title_label` keeps a divider title — the padding goes first, then the title takes `…`. `ViewportOverflow::for_paged_content` counts drawn lines, so a page mark names the drawn page and the last page draws no `▼`.
- A deleted project: Languages names the deleted state, and Targets draws no row for a deleted package and does not start its run.
- Counts and units stay whole, one description shows, a popup leaves no scraps behind it, `enter activate` shows only on rows that act on enter, and the CPU pane shows a page mark for hidden bars.
- CI runs at 80 columns hide `Branch` first in branch mode; the `Total` header ends on the edge of its values. The sccache `-` reason reads `Source read from standard input`. An open popup draws the thick focused border, also under `NO_COLOR`.

**Files:**
- `crates/cargo-port/src/tui/panes/project_list/` — column fit and `column_fit_tests.rs`
- `crates/cargo-port/src/tui/panes/package/render.rs` — two-column threshold and value wrapping
- `crates/cargo-port/src/tui/panes/targets/` — deleted-owner rows and launches, `availability_tests.rs`
- `crates/tui_pane/src/pane/rules.rs`, `crates/tui_pane/src/layout/viewport.rs` — divider titles and drawn page marks
- `crates/tui_pane/src/pane/popup.rs` — the focused popup border

**Binds later work:** the 60, 80 and 95 column fit tests, the 37-cell Package threshold, the divider title fallback and drawn-line page marks stay green. `target_rows_and_runs_follow_owner_deletion_and_recreation` and `workspace_targets_keep_live_owners_and_clamp_the_drawn_cursor` guard deleted-owner targets.

**Gotchas:** the Projects width and the Package threshold share one column budget; change one only with the fit tests.

**Ruled out:** a change to the Targets title; both title forms are correct.

### Phase 4e — cargo-port's popups and remaining words read clearly  · status: done

#### As-built

- Settings wraps paths at slashes inside the popup margin, selected Settings rows use one readable ink, and the Lints `Cache size` row shows its cap once (`1 B / 512 MiB`).
- Languages keeps meaningful names at normal width. An empty `Languages — deleted` pane gives focus away, as an empty Targets pane does.
- Package shows both CI counts and whole empty messages at narrow widths. Deleted metadata keeps its style and its true description state.
- Disk sizes use a measurement type with three states: not measured, measured (zero included), and a total with parts not yet measured. A not-measured project draws `…` in the Package disk rows, and an incomplete Projects sum draws `Σ …`.
- On a short pane (body under 12 rows) the Git description takes at most 2 rows and ends with `…`. In a narrow layout the Targets Kind column is measured from its real entries, and the freed cell goes to Source.
- Same-name project roots show the part of their paths that differs.
- The startup toast stays inside the pane at its placement corner at every row count. `tui_pane`'s shared card renderer clears only the card and the plain gap cells, keeps pane borders and titles in the gap rows, and checks the right-edge border only over the occupied rows. cargo-port makes one shared toast render call, with no frame clone and no screen scan.

**Files:**
- `crates/tui_pane/src/toasts/render/card.rs` — corner region choice, gap clearing and border keep
- `crates/tui_pane/src/toasts/render/drawing.rs` — drawn toast tests over framed panes
- `crates/cargo-port/src/tui/render/mod.rs` — the one shared toast render call
- `crates/cargo-port/src/tui/panes/` — pane text and the per-pane `readability_tests.rs`
- `crates/cargo-port/src/tui/settings_readability_tests.rs` — Settings popup tests
- `crates/cargo-port/src/project/`, `crates/cargo-port/src/watcher/`, `crates/cargo-port/src/scan/background.rs` — the disk measurement type and its messages

**Binds later work:** the toast width comes from the corner pane rect, not the frame. `a_bottom_right_stack_stays_below_the_pane_border_when_rows_are_hidden`, `pane_borders_outside_the_card_keep_the_unrestricted_toast_rect` and cargo-tile's `hidden_grid_contents_leave_the_toast_at_its_unrestricted_rect` stay green. The two-row description cap below 12 body rows stays.

**Gotchas:** an emoji font draws its own colours, so the ink setting does not change an emoji glyph.

**Ruled out:** text glyphs in place of the status emoji on selected rows; an emoji is a picture, not ink on a ground.

### Phase 4g — The rate-limit countdown never stops at 0s  · status: done

#### As-built

- At a GitHub quota reset the app wakes with no input, fetches `/rate_limit`, and invalidates the Git data so the pane redraws. When the reply still carries the old window, `quota_refresh.rs` fetches again every 10 s (`RESET_RETRY_INTERVAL`), at most 3 times (`ResetRetries::{One, Two, Three}`, `ResetRetrySchedule::{Due, Exhausted}`), then waits for the regular poll. A reset costs up to 4 quota-exempt requests, typically 2.
- A rate-limit row shows `<remaining>/<limit>` when it fits and adds `resets <time>` when that fits too. A reset at or before now shows no `resets` text.
- `GitData` carries a GitHub applicability type (not GitHub, pending, measured, unreachable) to the render. An outage shows one `GitHub unreachable` row in one colour; a package outside GitHub has no `Stars` row.
- Idle draw suppression holds: nothing draws while nothing changed.

**Files:**
- `crates/cargo-port/src/tui/app/async_tasks/quota_refresh.rs` — the reset wake and the retry schedule
- `crates/cargo-port/src/tui/app/async_tasks/quota_completion_tests.rs`, `crates/cargo-port/src/tui/app/quota_transition_tests.rs`, `crates/cargo-port/src/tui/terminal/quota_wake_tests.rs` — fake-clock reset, completion and wake tests
- `crates/cargo-port/src/tui/panes/pane_data/` — the rate-limit text, the applicability type and `rate_limit_tests.rs`
- `crates/cargo-port/src/tui/panes/git/render.rs` — the outage row and the rate-limit fit
- `crates/cargo-port/src/tui/terminal/event_loop.rs`, `crates/cargo-port/src/tui/state/net.rs`, `crates/cargo-port/src/tui/startup_services.rs` — the wake and fetch wiring
- `crates/cargo-port/CHANGELOG.md`

**Binds later work:** the reset retry (fetch at the reset time, then at most 3 retries 10 s apart) and its quota presentation stay as they are. Git pane work builds on the GitHub applicability type.

**Gotchas:** GitHub's `/rate_limit` can return the old window for seconds after its reset time; the retry exists for this.

**Ruled out:** one fetch at the reset time only; a stale countdown until the next regular poll.

### Phase 4h — The startup toast marks hidden rows with a page mark  · status: done

#### As-built

- A toast with more rows than fit reports its hidden rows through the panes' `ViewportOverflow` and draws the `▼` page mark in its bottom border, as the panes do. Plain and tracked bodies both report; no row ends in a lone `…`. The tracked body keeps its `(+N more)` row with the exact count. The mark never covers the countdown or the action hint.
- cargo-port passes the interior of the pane at the placement corner as the allowed toast area, so the toast is no wider than that pane and the detail column is cut first. At 80x30 the pane borders stay whole.
- An opening card keeps one blank row above the pane border when it fits, and the page count stays fixed while the card opens. Each progress row cuts its label beside its own suffix only.
- The startup toast builds its rows once per frame, after layout.

**Files:**
- `crates/tui_pane/src/toasts/render/card.rs` — card layout, the overflow report and the bottom-border mark
- `crates/tui_pane/src/toasts/render/overflow_tests.rs` — drawn plain and tracked overflow tests in colour and NO_COLOR
- `crates/cargo-port/src/tui/render/mod.rs` — the corner pane interior as the allowed toast area
- `crates/cargo-port/src/tui/render/readability_tests.rs` — the 80x30 toast bounds tests
- `crates/tui_pane/CHANGELOG.md`

**Gotchas:** the `(+N more)` row is the tracked body's own overflow report; tests pin its count and its place as the last body row.

**Ruled out:** a scan of drawn border glyphs to find the toast area; the caller passes the pane rect.

### Phase 4i — Selected-row status glyphs read at 4.5:1  · status: dropped (showrunner, 2026-11-07: "an emoji is a coloured picture, not ink on a ground, so the 4.5:1 text bound does not apply to it"; the emoji stay as they are on selected rows)

### Phase 4j — Package, Git and Targets rows read clearly at narrow widths  · status: done

#### As-built

- The GitHub run count is `GithubRunCount` (`Loading`, `Failed`, `Known(n)`) in `tui/state/ci.rs`. Both CI fetch functions in `scan/ci_cache.rs` classify the list reply through one function, and the count passes through `ProjectCiInfo`, `CachedRepoData` and `BackgroundMsg::CiRuns`. A known count stays through a later failed fetch.
- The Package CI row draws the status icon and `local N`; the next row draws `gh N`, `gh …` while the count loads, or `gh ?` after a failed fetch. The `gh` row starts at the drawn column of `local`, so the icon width does not move it. With no next row, only the CI row draws. `CiDisplay::NoRuns` draws `No CI runs` for a known zero here and on GitHub.
- The Git Worktrees and Remotes tables share one column layout (`sync_col_layout`, `SyncTableMode` in `git/render.rs`). Name, Branch, Tracked and Sync stay at narrow widths: Name is cut first, every column keeps at least one letter and `…`, and a value that fits draws whole. Headers may be cut when four whole headers do not fit. Each row keeps one blank cell at each edge, and columns pad by display width.
- The Targets Source of same-name worktrees shows the differing part of each path through `grouping::differing_path_labels`, the code of the Projects pane. When two labels read the same after the width cut, `fit_distinct_path_labels` cuts in the middle. A worktree source gives the Source column a floor of its header width; each target keeps its whole name before Source takes width. Launch paths do not change.
- Package disk rows show `…` and the Projects sum `Σ …` until the disk scan measures the project.

**Files:**
- `crates/cargo-port/src/tui/state/ci.rs` — `GithubRunCount`, `CiDisplay`
- `crates/cargo-port/src/scan/ci_cache.rs` — the CI list classification
- `crates/cargo-port/src/tui/panes/package/render.rs`, `package/ci_count_tests.rs` — the CI rows and their drawn tests
- `crates/cargo-port/src/tui/panes/git/render.rs`, `git/sync_column_tests.rs` — the shared sync table layout
- `crates/cargo-port/src/tui/panes/targets/render.rs`, `crates/cargo-port/src/tui/project_list/grouping.rs` — the Source labels
- `crates/cargo-port/CHANGELOG.md`

**Gotchas:** an end cut can make two differing path labels read the same; the middle cut is the guard. `fit_sync_trio` cuts Tracked to its floor before Branch.

**Ruled out:** a placeholder exception to the right-margin cell at a 15-cell interior; one margin rule covers every Package row.

### Phase 4k — Loading and empty values read whole in narrow panes  · status: done

#### As-built

- Each GitHub quota bucket is a `QuotaObservation` (`Unobserved`, `Observed(RateLimitQuota)`) in `GitData` and `GitDetailFields`. `build_git_detail_fields` converts the rate-limit cache `Option` once. `git_fields_from_data` adds a Core or GraphQL row only for an observed bucket, and `format_rate_limit_bucket` takes an observed quota, so a drawn quota row never has an empty value.
- `HttpClient::set_rate_limit` keeps each cached bucket that a reply omits (`github_rate_limit.core.or(state.core)`), so a failed or partial refresh does not erase a known quota.
- The empty Git pane of a project that is not a git repository is always titled `Git`; its body stays empty until quota data arrives.
- ` No description` takes one row at a 16-cell Package interior and wraps to `No` / `description` at 15 cells, inside one inset cell on each side.
- A toast keeps one blank gap row above the pane's bottom border while it opens when the pane has `MIN_PANE_ROWS_WITH_EDGE_GAP` (6, `tui_pane/src/constants.rs`) rows or more. Page capacity comes from the allocated height, not the animated height.

**Files:**
- `crates/cargo-port/src/tui/panes/pane_data/` — `QuotaObservation`, the quota rows, `rate_limit_tests.rs`
- `crates/cargo-port/src/http/client.rs` — `set_rate_limit`
- `crates/cargo-port/src/tui/panes/git/render.rs` — `fit_rate_limit_value` and the empty Git title
- `crates/cargo-port/src/tui/panes/package/readability_tests.rs` — the 15- and 16-cell description tests
- `crates/tui_pane/src/toasts/render/` — `drawing.rs` (the gap row), `opening_tests.rs`, `test_support.rs` (the shared toast test app)
- `crates/cargo-port/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`

**Gotchas:** under NO_COLOR a focused pane draws heavy borders (`┗`), so a capture script that finds page counts by the border must match both border sets.

**Ruled out:** a `Not a git repo` title for the empty Git pane; a pane title does not change when data arrives.

### Phase 4l — Labels, titles and columns read whole in narrow panes  · status: done

#### As-built

- Labels never cut; names, values and counts take the ellipsis. Git section titles (`git/render.rs::section_title_with_count`) keep the label and cut the count first (`Worktrees (3)`), and Git field labels such as `Ahead/Behind` draw whole.
- The Targets pane keeps its Source column for workspace and worktree sources; under user rule C205 Source takes the ellipsis before a target name.
- Pane titles keep their label and cut the project name. `GridLines::write_overlay` (`tui_pane/src/pane/frame.rs`) draws a title that fits without its trailing space whole.
- Startup rows from `progress_panel_body` (`tui_pane/src/progress/render.rs`) keep each label whole and shrink the bar and count first.
- `CiEmptyState` titles fit at 80 columns: `CI runs — no GitHub remote`, `CI runs — not a git repo`, `CI runs — on branch rows`; `NoRunsForBranch` cuts only the branch name.
- Package, Workspace, CI runs and Languages rows keep one right-margin cell (`support::field_value_lines`, `package/constants.rs::PATH_VALUE_RESERVE`) and one blank cell before the Package `Structure` divider. A Package label shares a row with its value when both fit; values share one value column, not right-aligned. Package Disk children (`target/`, `Other`) share one indent, which shrinks only when a label and its value then share a row.
- `fit_sync_trio` cuts the wider of Branch and Tracked one cell at a time after Name reaches its floor, so a short Tracked value draws whole. At 80 columns all four Worktrees and Remotes columns stay, each with at least one letter and `…`.
- Every toast keeps one blank cell before `[x]`; at the widest card an error toast's `[x]` may use the last border cell before the corner so the title stays whole. A visible toast's body is tall enough for its first sentence before it adds pages; `ToastExitState { Visible, Exiting }` (`tui_pane/src/toasts/view.rs`) keeps an exiting toast at its height.

**Files:**
- `crates/cargo-port/src/tui/panes/targets/` — the Source floor
- `crates/cargo-port/src/tui/panes/git/` — section titles, field labels, `fit_sync_trio`, `git/readability_tests.rs`, `git/sync_column_tests.rs`
- `crates/cargo-port/src/tui/panes/package/` — the Package title, value margin, Disk indent, `package/readability_tests.rs`
- `crates/cargo-port/src/tui/panes/support.rs` — `field_value_lines`
- `crates/cargo-port/src/tui/panes/ci/` — the CI title words and the CI runs margin
- `crates/cargo-port/src/tui/panes/lang/` — the Languages `Total` margin
- `crates/cargo-port/src/tui/app/async_tasks/startup_phase/toast_bodies.rs` — Startup label rows
- `crates/tui_pane/src/progress/render.rs`, `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/pane/title.rs` — Startup rows and the title fit
- `crates/tui_pane/src/toasts/render/` — the `[x]` gap and first-sentence pages, `sentence_tests.rs`
- `crates/cargo-tile/src/render.rs` — the toast frame test expects the `[x]` gap
- `crates/cargo-port/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`

**Binds later work:** a loading Git section draws no count with the same whole-label rule; the Structure width reserve keeps the right-margin cell and the blank cell before the divider; the Package and Git readability tests stay green.

**Gotchas:** a tui_pane look change can break frame tests in other crates (`cargo-tile`), so run the cargo-tile and cargo-handler suites when a shared toast or pane frame changes.

**Ruled out:** cutting target names before Source; right-aligned Package values.

### Phase 4m — Panes keep their layout while data loads  · status: done

#### As-built

- The Workspace description is a `DescriptionState::{Pending, Known, KnownEmpty}`, carried through the Cargo and GitHub producers (`version_and_description`, `build_git_detail_fields`) to the height (`pane_data/detail_data.rs`) and the row (`panes/description.rs`). ` No description` draws only when both sources are known and empty; loading and a failed fetch draw no row, so the page count changes only when a row with real data first appears.
- `GitRepo` holds a `RepositoryProbe::{Awaiting, NoRepository, Repository}`. Discovery and the background scan emit `NoRepository` for a project outside git (`repo_handlers.rs::handle_no_repository`), so its description resolves.
- The Package `Structure` column measures every crates.io label before the data arrives and gives values a fixed 8-cell budget; a longer value (a long version, `Unreachable`) is cut with `…` in `render_stat_section`. The divider does not move.
- The CPU / GPU page count draws in the bottom border through the shared border pager.
- The Git `Pull Requests` title shows a count only when it is known: no count while loading or when `Unavailable`; stale data keeps its last count.
- A cut Package title keeps one blank cell before its corner (`package/render.rs::fit_package_title`); a whole label that fills the inner width may touch the corner.

**Files:**
- `crates/cargo-port/src/project/entry.rs` — `RepositoryProbe`
- `crates/cargo-port/src/scan/background.rs`, `scan/discovery.rs` — emit `NoRepository`
- `crates/cargo-port/src/tui/app/async_tasks/repo_handlers.rs` — `handle_no_repository`
- `crates/cargo-port/src/tui/panes/description.rs`, `pane_data/detail_data.rs` — description states and height
- `crates/cargo-port/src/tui/panes/package/render.rs` — Structure width, `fit_package_title`
- `crates/cargo-port/src/tui/panes/cpu/`, `panes/git/render.rs` — border pager, title count
- `crates/cargo-port/src/tui/app/loading_layout_tests.rs` — drawn loading-state tests

**Binds later work:** the description states, `RepositoryProbe::NoRepository`, the fixed Structure width and the cut-title rule stay as built (Cut values and dim text read clean in narrow panes).

**Gotchas:**
- A project with no repository must still get a finished probe result; without it, "known empty" never resolves.
- `GithubDescriptionSource` (`pane_data/git_data.rs`) repeats the remote-URL parse of `fetch_url_for`.

**Ruled out:** a title rule that always keeps a blank cell before the corner: a whole label stays whole (showrunner, 2026-11-07).

### Phase 4p — cargo-port memory stays bounded over days  · status: done

#### As-built

- Source measurement caches (`scan/source_measurements.rs`) parse each file once, skip unchanged files and drop retired owners. A late job captures its cache before its async work, so it cannot restore a retired owner.
- `WorkspaceMetadataStore` and its dispatch generations drop retired checkouts. `TargetDirIndex` replaces a checkout's members on each metadata result.
- A metadata refresh keeps one pending refresh per checkout (`scan/pending_work.rs`) and takes a run permit per refresh. Rescans for one root merge.
- One shared worker pool (`scan/background_workers.rs`) runs background work. Callers run `rayon::spawn(scan::project_detail_job(&request))`; `enrichment_job` and `language_enrichment_job` build the enrichment work.
- The watcher removes retired projects from `projects`, `project_parents` and `pending_disk`. `WatchSkipSet` returns an exclusion-transition enum.
- On GNU/Linux, `main.rs` calls `mallopt(M_ARENA_MAX, 2)` at startup, so freed scan buffers return to use.
- A driven 600 s run holds 192-194 MB, flat after warm-up; before this phase the same run reached 1.43 GB.

**Files:**
- `crates/cargo-port/src/scan/` — caches, pending work, worker pool, detail and enrichment jobs
- `crates/cargo-port/src/watcher/` — project retirement and skip transitions
- `crates/cargo-port/src/main.rs`, `crates/cargo-port/src/constants.rs` — arena limit

**Binds later work:** background work runs on the shared pool in `scan/background_workers.rs`, never a thread per call. The bound tests `metadata_retention_tests`, `disk_burst_tests`, `skip_growth_tests` and `project_retirement_tests` stay green.

**Gotchas:**
- Live heap stays flat while glibc arenas grow the total; measure `VmRSS` plus `VmSwap`, and split live heap from allocator pages before a hunt for a growing structure.
- Startup project-detail jobs read `.gitmodules` (and run `git ls-tree` for a submodule project) on the caller thread, so caches are captured before dispatch. This costs about 6 ms per submodule project.

**Ruled out:** a size cap on a structure that should not grow; it hides the growth.

### Phase 4q — The project list uses the shared row click toggle  · status: done

**Depends on:** Phase 4p (both edit `crates/cargo-port/src/`), and the handler unit's shared row click toggle in `tui_pane` on main (the showrunner sends its hash). It runs after Phase 4t (showrunner, 2026-11-09).

#### As-built

A left click on a project row goes through `tui_pane::ExpansionModel::press_row`. `ProjectList` keeps the expansion state. `ProjectList::press_row(row, NonRustInclusion) -> RowPressResult` calls the shared model and runs `recompute_visibility` only after `RowPressResult::Toggled`. A focus-only click (`ClickMode::FocusOnly`) does not toggle, and a test in `tui/app/mod.rs` pins it. cargo-port has no click toggle of its own, and `tui/project_list/selection.rs` does not exist.

**Files:**
- `crates/cargo-port/src/tui/project_list/list.rs` — `ProjectList::press_row`
- `crates/cargo-port/src/tui/interaction.rs` — the click handler that calls it
- `crates/cargo-port/src/tui/app/mod.rs` — the click tests

**Gotchas:** `ProjectList::press_row` takes `expanded` out with `std::mem::take` for the call and puts it back, because the shared model borrows the model and the row source at once.

**Ruled out:** a cargo-port copy of the shared toggle; a change the shared piece needs goes to its owner.

### Phase 4n — Targets, cut titles and the footer read clean in narrow windows  · status: done

#### As-built

- `panes/layout.rs` reserves the preferred Targets width (the longest target name plus Source and Kind) before Projects grows, when every middle pane fits. At 160 columns target names draw whole.
- Targets Source shrinks to a two-cell floor (one letter and `…`) for every source kind before a target name is cut; the Source header cuts the same way (`S…`).
- A cut pane title keeps one blank cell before its corner or column joint (`tui_pane` `GridLines::write_overlay`).
- `tui_pane::StatusLine` has a private `HintFit::{WholeGroups, IndividualHints}` policy. `WholeGroups` is the default: whole hint groups, as for cargo-tile and cargo-handler. `StatusLine::with_individual_note_hints()` selects `IndividualHints`: on a row with notes, hints drop one at a time from the right at separator boundaries and at least 3 blank cells stay before the notes. cargo-port opts in (`render/mod.rs::render_status_bar`).

**Files:**
- `crates/cargo-port/src/tui/panes/targets/render.rs`, `targets/pane.rs`, `targets/constants.rs` — Targets width and Source cut
- `crates/cargo-port/src/tui/panes/layout.rs` — Targets preferred width
- `crates/cargo-port/src/tui/panes/targets/width_tests.rs` — width tests at 95 and 160
- `crates/tui_pane/src/pane/frame.rs` — cut title gap
- `crates/tui_pane/src/bar/status_line.rs`, `status_line_gap_tests.rs` — hint fit policy and its tests

**Binds later work:** the Targets preferred width, Source shrinking first, the cut-title gap and the opt-in footer fit stay as built (Cut values and dim text read clean in narrow panes). That phase adds the cargo-port changelog entry for the Targets width.

**Gotchas:**
- `tui_pane` is shared with cargo-tile and cargo-handler. cargo-tile shows its name and version as a status line note, so a fit rule keyed only on "notes exist" changes its footer; the hint fit stays opt-in. The cargo-tile render tests catch it.
- Targets used `Constraint::Min`, so Projects took its width at 160 columns until the preferred width was reserved.

**Ruled out:** a live shot of `CI runs — on branch rows`: Cut values and dim text read clean in narrow panes removes that title.

### Phase 4o — Cut values and dim text read clean in narrow panes  · status: done

#### As-built

- `render::truncate_with_ellipsis` (`render/mod.rs`) trims trailing spaces and a trailing ` ·` separator before it adds `…`. The CI, Git, Package and Targets values use it.
- The `text.dim_value` theme role (`tui_pane` `DIM_VALUE_ROLE`, `dim_value_color()`; secondary text is the fallback) draws `No CI`, `No CI runs`, `unpublished branch` and the lint `—`. Every registered cargo-port theme gives it at least 4.5:1 on the focused and the unfocused pane background. Borders keep `inactive_border`.
- The Projects columns reserve five cells on Main before comparisons arrive (`MAIN_COMPARISON_VALUE_RESERVE`, `tui/columns/constants.rs`); Sync is `Fit { min: 0 }`. The Projects/Git divider does not move when Sync or Main data arrives; a wider value grows the column once. The `Projects` header starts on the name start cell, and a cut tree name keeps one letter before `…`.
- Git: `fit_sync_trio` keeps `Tracked` whole at 95 columns, where the Git pane is 23 cells. Field labels keep both margins when `label + 2` fits; a short Head or Branch value fits inline.
- The Worktree Group `CI` and `No CI` share one row in the value column at 80 columns.
- Pull requests (`PullRequestSectionState`, `pane_data/git_data.rs`): an unfetched list that will be fetched draws `Loading` from the first frame, so the Git page count does not change when the row appears. Fetch results reach submodule entries (`replace_pr_data_for_repo`, `project_list/list.rs`). A repo with an unreadable remote list and no cached result hides the section.
- `CiEmptyState::BranchScopedOnly` and its ` CI runs — on branch rows ` title are gone.
- `test_support/readability.rs` has a theme override fixture whose `Drop` restores the theme, and one shared check for no space before `…`.

**Files:**
- `crates/cargo-port/src/tui/render/mod.rs` — cut with ellipsis
- `crates/cargo-port/src/tui/columns/constants.rs`, `columns/project.rs` — Main reserve
- `crates/cargo-port/src/tui/panes/git/render.rs` — label margins, Tracked fit
- `crates/cargo-port/src/tui/panes/pane_data/git_data.rs` — pull request section state
- `crates/cargo-port/src/tui/project_list/list.rs` — submodule pull request delivery
- `crates/cargo-port/src/tui/panes/ci/data.rs`, `ci/render.rs` — CI empty states
- `crates/cargo-port/src/tui/theme_roles.rs`, `crates/tui_pane/src/theme/` — dim value role
- `crates/cargo-port/src/tui/test_support/readability.rs` — theme override fixture and the ellipsis check

**Binds later work:** the Main reserve, the `Projects` header on the name start cell, one letter before `…` in a cut name, and no space or ` ·` before `…` stay as built (The project list uses the shared row click toggle; Remaining cut and contrast defects read clean). The older theme-override tests move to the `Drop` fixture in Remaining cut and contrast defects read clean.

**Gotchas:**
- Measure text contrast against the real pane backgrounds, focused and unfocused, not white: Default Light dim text passed on white and failed on the focused pane.
- At 95 columns the Git pane is 23 cells, not 24.
- A theme test that builds one App per case takes over 1 s; build one App and loop over the themes.

**Ruled out:** a Sync-column reserve (it moved the Main header and cut tree names to no letters at 80 columns).

### Phase 4r — mend --fix keeps the tui_pane tiles re-exports and writes no empty use  · status: done

#### As-built

- `LocalImportRelation::for_paths` (`fixes/imports/path.rs`) gives `Descendant` only when the target path is longer than the common prefix. A self-module import, such as `use crate::tiles::grid;` inside `tiles::grid`, is not a descendant with an empty suffix, so mend never writes `use ;`.
- That one cause also broke the Phase 31 root re-exports. With it fixed, `use crate::TileDemand` and `use crate::TileContent` resolve through `lib.rs`'s `pub use tiles::…`.
- `ReexportMountInventory::collect` (`fixes/subtree_reexport/scan.rs`) reuses `selection.packages` when the selection's manifest is the workspace root `Cargo.toml`. A member-directory run still runs its own workspace lookup. A re-export chain fix runs one `cargo metadata`, not five.
- On a copy of the cargo-tile Phase 31 tree, `cargo mend --all-targets --workspace --fix` applies two fixes with no rollback, and the workspace check passes.

**Files:**
- `crates/cargo-mend/src/fixes/imports/path.rs` — local import relation and the empty-suffix guard
- `crates/cargo-mend/src/fixes/subtree_reexport/scan.rs` — the mount inventory reuses the selection's packages
- `crates/cargo-mend/tests/diagnostics/import_fixes.rs` — `fix_keeps_tile_root_exports_and_self_module_imports`
- `crates/cargo-mend/tests/diagnostics/pub_use_outside_subtree_fixes.rs` — `fix_chain_with_check_budget` asserts one `cargo metadata` run
- `crates/cargo-mend/CHANGELOG.md` — two Fixed entries

**Binds later work:** `fix_chain_with_check_budget` allows at most five `cargo check` runs and exactly one `cargo metadata` run; mend --fix collapses a re-export chain with fewer checks lowers the check budget. `reexport_chain_collapses_in_one_run` takes 2.7-3.1 s in the full suite before that phase.

**Gotchas:** the `for_paths` unit test row for a self-module import (`use crate::a::b;` inside `a::b`) expects `None`.

### Phase 4t — mend --fix collapses a re-export chain with fewer checks  · status: done

#### As-built

`cargo mend --fix` plans re-export chain links whose edits do not depend on a check result in one pass, and validates them with one `cargo check`. A chain costs 3 checks (5 before), the one-failed-link fallback costs 3 (4 before), and a run uses one `cargo metadata` and no rustfmt process.

- `ChainValidation::{Batch, PrunedBatch, IndividualLinks}` (`fixes/runner/chain.rs`) selects the validation path for one fix run.
- `StepAttribution::{SingleBinding, SharedBindings}` is set per planning pass. A failed batch prunes the links that compiler error files locate. A located `SharedBindings` step goes to `IndividualLinks`, and its pass limit is dynamic.
- `compiler/build/stderr.rs` reads `ErrorLocations` from cargo's rendered stderr and returns `error_files` on the build outcome.

**Files:**
- `crates/cargo-mend/src/fixes/runner/chain.rs` — chain validation paths and pruning
- `crates/cargo-mend/src/fixes/subtree_reexport/chain.rs` — chain link planning
- `crates/cargo-mend/src/compiler/build/stderr.rs` — error file locations
- `crates/cargo-mend/tests/diagnostics/pub_use_outside_subtree_fixes.rs` — `fix_chain_with_check_budget`, `failed_chain_link_keeps_the_other_validated_fixes`, `failed_chain_link_keeps_an_independent_chain`

**Gotchas:** `--message-format=json-render-diagnostics` makes cargo emit no compiler-message on stdout, so diagnostics come from stderr only.

**Ruled out:** named eligibility states for `public_use` and `use_path`, which are private syntax-match helpers where `None` means no match.

### Phase 4u — Every cargo-mend test runs under a second  · status: done

#### As-built

Every test in the full `verify.sh test cargo-mend` suite takes under 1 s; the slowest takes 0.902 s. The 4t chain and fallback tests take a median of 0.260 s and 0.241 s alone.

- `cargo mend` shares immutable driver copies across projects in `~/.cache/cargo-mend`. A run that cannot use the cache falls back to a local wrapper.
- Each cached run holds a shared lock on `retention.lock` for the whole run. A short exclusive `publication.lock` covers only the publish step. Eviction removes a copy only when its `try_lock` on the retention lock succeeds.
- Compiler-fix validation reuses earlier checks, rustfmt resolves once, and the progress thread exits with no shutdown delay.

**Files:**
- `crates/cargo-mend/src/compiler/build/execute.rs` — driver copy cache, retention and publication locks
- `crates/cargo-mend/src/compiler/constants.rs` — `WRAPPER_CACHE_RETENTION_LOCK_FILE` and `WRAPPER_CACHE_PUBLICATION_LOCK_FILE`
- `crates/cargo-mend/src/compiler/build/progress.rs` — progress thread shutdown
- `crates/cargo-mend/src/fixes/runner/function_length/` — short functions skip rustfmt

**Gotchas:** Measure a first run after a relink. Later runs reuse the cache and hide a lock wait.

**Ruled out:** One exclusive lock for publish and retention, because the publisher waits on every running mend.

### Phase 4s — Remaining cut and contrast defects read clean  · status: done

#### As-built

At 80 and 95 columns the Targets `Source` header draws whole while a target name cuts. Every Package `—` draws in `dim_value_color()`. The startup popup page footer meets 4.5:1 in every registered cargo-port theme.

- A cut ahead/behind value drops whole tokens: `↑1 ↓17` cuts to `↑1…`.
- `fit_package_title` keeps the whole label, or the label, the separator and at least one letter of the name.
- `fit_rate_limit_value` returns `RateLimitValue::{Inline, Stacked}`. It keeps seconds only when the widest seconds form fits inline, otherwise it shows whole minutes, and under one minute it shows seconds. It drops the reset only when the reset does not fit `area_width - 2`.
- Every test that overrides the theme uses the `ThemeOverride` Drop fixture.

**Files:**
- `crates/cargo-port/src/tui/panes/git/render.rs` — `RateLimitValue`, `fit_rate_limit_value`
- `crates/cargo-port/src/tui/panes/pane_data/formatting.rs` — whole-token ahead/behind fit
- `crates/cargo-port/src/tui/panes/package/render.rs` — dim placeholders, `fit_package_title`
- `crates/cargo-port/src/tui/panes/targets/render.rs` — `Source` header priority over names
- `crates/tui_pane/src/toasts/render/card.rs` — toast page footer colour
- `crates/cargo-port/src/tui/theme_roles.rs` — `startup_page_footer_meets_contrast_in_every_registered_theme`
- `crates/cargo-port/src/tui/test_support/readability.rs` — `ThemeOverride`

**Gotchas:** A rate row shape must not change as the countdown runs, so precision and shape are selected in one function.

**Ruled out:** A rate value cut inline at 80 columns, because the value stacks under its label instead.

### Phase 4v — Returned narrow-width, border and lint column fixes  · status: done

**Depends on:** Phase 4s (both edit `crates/cargo-port/src/tui/panes/`). It runs after Phase 4s (showrunner, 2026-11-09).

#### Work Order

**Goal:** at 60 and 80 columns a project name gives way before any list column goes, Package labels and pane titles draw whole, the Git list keeps usable height, Sccache draws one border, and the Lint Runs and CI runs tables show their columns.

**Source:** showrunner validation of C222-C229, 2026-11-09. C222, C224 and C227 return; items 4 and 5 are new defects.
1. C222: at 60 columns Type, Git, Sync and Main drop while names grow: `│   Projects    CI      Disk │`, `│ ▶ bevy-0.20.0    142.1 MiB │`. The user's rule: names get `…` before any column goes.
2. C224: at 80 columns a Package label is cut (`│  integ… 1294  │`, page 3) and the pane title is cut (`┌Worktree Gr… ┐`). With 13 worktrees the Git list area is one row tall (`▲ 18 of 28 ▼`) and the divider has no `Worktrees (N)` title.
3. C227: NO_COLOR draws the thick Sccache border; color mode draws a thin one (`┌ Sccache stats ───┐`).
4. Lint Runs columns are cut at every width. 160: `│2026-10-  19:00:55  19:00:55`. 80: `ration`, `2026-1`. 60: `art  End  ion  ize  Res`.
5. At 80 columns under NO_COLOR, `CI runs (466)` (not branch mode) shows only Commit, with no Total column.
6. Phase 4s final design check of `/tmp/claude/unit/a531f8d0-5a0c-483d-aa99-3738f735f9a7/phases/4s/live/shots_4s_3/`: in `quota80-*-loading-git-2_of_3`, the popup title draws `! crates.io unreacha…` in the 29-cell card. It needs 2 more cells. `! GitHub unreachable` draws whole.

**Decision:** showrunner, 2026-11-09: one fix phase for all five, after Phase 4s. Each fix returns as a new Change-Review row. Item 6 moved here from Phase 4s under the landing rule: it is older than that phase and outside its Spec. Rulings from the design check, showrunner, 2026-11-09: a worktree group row keeps `▶ `, the letters that fit and the tree, and drops the count when the slot is short; the Git list draws only the columns whose whole labels fit; the older defects move to Phase 4x. Showrunner, 2026-11-09, on the list labels: from 60 to 80 columns Disk drops its decimal, the name column gets 8 cells, and the labels line up with Lint off.

**Constraints:**
- Never cut a label. Cut a value, a name or a message first.
- Colours come from the theme, not from code.
- Keep the Phase 4o rules: no space or ` ·` before `…`; the five-cell Main reserve; the `Projects` header on the name start cell; a cut name keeps one letter.
- Change no other look at widths where nothing is cut now.

**Spec:**
1. **Project list columns.** At 60 columns a name gets `…` before Type, Git, Sync or Main drops (`tui/columns/`, `tui/panes/project_list/`).
2. **Package and Git.** At 80 columns no Package label or pane title is cut. With 13 worktrees the Git list area keeps more than one row, and the divider carries `Worktrees (N)` (`tui/panes/package/`, `tui/panes/git/`). `git_lower_content_height` (`panes/git/render.rs`) omits the stacked rate rows, and `pane_data/detail_data.rs` shares that estimate: count those rows in both. Replace the two `Option<Rect>` fields of `GitAboutLayout` with a named type for its three outcomes: no description, a description with no separator space, and a description with a separator. Convert at the drawing and viewport boundaries.
3. **Sccache border.** Draw the thick border in color mode and under NO_COLOR (`tui/sccache/render.rs`). `PopupFrame::render` in `tui_pane/src/pane/popup.rs` selects the border, and the shared tests require a plain coloured border. Add a named border policy that Sccache selects. Every other popup keeps its border.
4. **Lint Runs.** At 60, 80 and 160 columns no Lint Runs header or date is cut inside a word (`tui/panes/lints/render.rs`).
5. **CI runs.** At 80 columns under NO_COLOR, outside branch mode, the CI runs table shows a Total column (`tui/panes/ci/`).
6. **Popup title.** At 80 columns the popup title `crates.io unreachable` draws whole. Keep `[x]` and the card border. A longer title still cuts with `…`. (`crates/tui_pane/src/toasts/render/card.rs`). `toasts/render/drawing.rs` caps the card width before `card.rs` fits the title, so the fix covers that width too. Extend `an_error_title_grows_before_it_uses_an_ellipsis` (`toasts/render/repair_tests.rs`, now a 32-cell area) with the recorded 29-cell layout, a whole title and a working close hit area.
7. **Changelog.** Add the cargo-port and `tui_pane` entries.

**Constraints from prior phases:**
- Keep the Phase 4s rules: the Targets `Source` header draws whole before a name cuts; a cut ahead/behind value drops whole tokens; every Package `—` uses `dim_value_color()`; `fit_package_title` never leaves ` - ` before `…`.
- A Git `Core` or `GraphQL` value that does not fit inline takes two rows. `fit_rate_limit_value` keeps the row shape steady as the reset counts down.
- A new test that overrides the theme uses the `ThemeOverride` fixture (`tui/test_support/readability.rs`).
- Keep `startup_page_footer_meets_contrast_in_every_registered_theme` (`tui/theme_roles.rs`) green: Spec 6 edits the file that draws the footer.

**Files:**
- `crates/cargo-port/src/tui/columns/` and `crates/cargo-port/src/tui/panes/project_list/` — Spec 1
- `crates/cargo-port/src/tui/panes/package/`, `crates/cargo-port/src/tui/panes/git/`, `crates/cargo-port/src/tui/panes/pane_data/detail_data.rs` and `crates/cargo-port/src/tui/render/readability_tests.rs` — Spec 2
- `crates/cargo-port/src/tui/sccache/render.rs` and `crates/tui_pane/src/pane/popup.rs` — Spec 3
- `crates/cargo-port/src/tui/panes/lints/render.rs` — Spec 4
- `crates/cargo-port/src/tui/panes/ci/` — Spec 5
- `crates/tui_pane/src/toasts/render/card.rs`, `crates/tui_pane/src/toasts/render/drawing.rs` and `crates/tui_pane/src/toasts/render/repair_tests.rs` — Spec 6
- `crates/cargo-port/CHANGELOG.md` and `crates/tui_pane/CHANGELOG.md` — Spec 7

**Workers:** 2 writers.
- Slot 1: Specs 1, 2, 4 and 5, their tests and `crates/cargo-port/CHANGELOG.md`, with the Spec 3 line.
- Slot 2: Specs 3 and 6, their tests and `crates/tui_pane/CHANGELOG.md`.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-port`, `verify.sh test tui_pane` and `verify.sh lint cargo-port` clean; no test takes a second. Drawn tests pin each of the six defects at the width that showed it. The Git test uses 13 worktrees, stacked rate rows and a description, and it scrolls. Live captures at 60, 80 and 160 columns in colour and under NO_COLOR, judged by the design check. The checkpoint carries one `Change-Review:` line per fix.

### Phase 4w — Targets lists a binary whose name differs from its package  · status: todo

**Depends on:** Phase 4s (it edits `crates/cargo-port/src/tui/panes/targets/`).

#### Work Order

**Goal:** a package whose binary has a different name from the package lists that binary in Targets and does not show `No Targets`.

**Source:** live capture for Phase 4s, 2026-11-09. `TargetsData::from_workspace_metadata` (`panes/targets/data.rs`) lists a `TargetKind::Bin` only when `target.name == record.name`, and `primary_binary_matches_package_name_only` asserts that rule. A package with only a differently named `[[bin]]` shows `No Targets` and has a runnable binary.

**Decision:** showrunner, 2026-11-09: this phase runs with its own selection-rule design.

**Constraints:**
- A package with one binary named for the package keeps its present row and launch.
- Do not change how examples and benches are listed.
- `primary_binary_matches_package_name_only` changes only as the approved rule states, with an `Assertion-Change:` line.

**Spec:**
1. **Selection rule.** Before any code, the unit director writes the rule and sends it to the showrunner. The rule covers a package with several binaries, `default-run`, a workspace member and a workspace root. Build only the approved rule.
2. **Listing.** `TargetsData::from_workspace_metadata` lists the binaries that the approved rule selects.
3. **Launch.** Each listed binary launches its own target (`--bin <name>` with its package).
4. **Changelog.** Add the cargo-port entry.

**Files:**
- `crates/cargo-port/src/tui/panes/targets/` — Specs 2 and 3
- `crates/cargo-port/CHANGELOG.md` — Spec 4

**Workers:** 1 writer: Specs 2-4 and their tests.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second. Metadata and drawing tests show the selected binaries in place of `No Targets`. A launch test selects the correct target. A live capture of a package with a differently named `[[bin]]`, judged by the design check. The checkpoint carries a `Change-Review:` line.

### Phase 4x — CI job header and table margins read clean  · status: todo

**Depends on:** Phase 4v (both edit `crates/cargo-port/src/tui/panes/ci/`, `panes/lints/` and `sccache/`).

#### Work Order

**Goal:** a CI job header draws whole at 160 columns, the Lint Runs, CI runs and Sccache rows keep the one-cell margin that other tables keep, and the older narrow and wide pane looks from the Phase 4v design check read clean.

**Source:** Phase 4v design check, `/tmp/claude/unit/a531f8d0-5a0c-483d-aa99-3738f735f9a7/phases/4v/live/shots_4v_2/`. These defects are older than Phase 4v and outside its Spec.
1. `w160h45-*-2-main-checkout`: the CI runs header reads `Windows Bui…`, and the Commit values in the same table are 26 cells wide.
2. `*-1-lint-runs`, and `w60h30-*-2-main-checkout` and `-2-ci-all`: text touches the left border (`│Date`, `│24m 49s 🔴   │`). Other tables keep one cell (`│ Commit`).
3. `w80h*-*-4-sccache`: the hint row ends `pageup/pagedown page┃` and touches the right border. At 160 columns the row keeps one cell.
4. Phase 4v final design check, `/tmp/claude/unit/a531f8d0-5a0c-483d-aa99-3738f735f9a7/phases/4v/live/shots_4v_3/w80h30-color-2-main-git-4_of_4.txt`, rows 3-4: a plain divider sits on the `Remotes` divider (`├───┤`, then `├─ Remotes (1) ─┤`). Phase 4s frames show the same pair.
5. `w160h45-*-2-ci-all`, row 37: a commit value cuts directly after a separator (`phase 32d —…`).
6. `w60h30-*-5-lint-on` and `w80h30-*-5-lint-on`: with Lint on, the column labels sit 3 cells right of their values after the Phase 4v Disk change.
7. Phase 4v final design check, `/tmp/claude/unit/a531f8d0-5a0c-483d-aa99-3738f735f9a7/phases/4v/live/shots_4v_5/`, `w80h45-*-7-member-*`: the Git description takes 11 rows and Package pads to match, so a taller window shows fewer fields than the 30-row window.
8. `w60h30-*`, `w80h*-*` and `quota80-*`: the group row keeps the count while the name has one letter (`▶ m… 🌲:13`).
9. `w80h30-*-7-member-*`: `target/` and `0 B` take two rows when one row fits, and the `target/` indent changes between frames.
10. `w160h45-*-1-lint-runs` and `-6-lint-runs`: the date column has no header at 160 columns.
11. `w160h45-*-1-lint-runs`: the Lint value sits 2 cells right of its column because the empty icon slot draws.
12. `w80h*-*`, `quota80-*` and `w60h30-*-5-lint-on`: a title with one spare cell has a left space and touches the right corner (`┌ Package - alpha┐`).
13. `w60h30-*`, `w80h*-*` and `quota80-*` Git pages: a section rule sits directly under the top border (`┌ Git - main ───┐`, then `├ Worktrees (13)┤`).
14. `w80h30-*-2-main-git-2_of_4` and `quota80-*-1-loading`: a Git date cuts inside the time (`2026-03-07 0…`). `w60h30-*-5-lint-on`: the date loses the day (`2026-11-…`) because 9 cells remain after Phase 4v keeps the list columns.
15. `quota80-*-1-loading-git-last-settled` and `-3_of_3`: messages cut with a free row below them (`GitHub unrea…`, `loading pull…`).
16. `w160h45-*-2-main-checkout`, `-2-ci-all` and `-2-main-git-*`: Languages and Targets names cut at 160 columns (`└ uni…`, `M↓ Markd…`, `cargo-… binary`).
17. Phase 4v checkpoint design check, `/tmp/claude/unit/a531f8d0-5a0c-483d-aa99-3738f735f9a7/phases/4v/live/shots_4v_6/`, `w80h*-*` and `quota80-*`: a cut Path value narrows its label column (`Path     /tmp…` at cell 10, `Lint       60` at cell 12).
18. `w80h*-*-7-member-*` and `w100h30-*-7-member-*`: cut member names are the same on several rows (`├── cargo…` six times at 80, `├── cargo-be…` twice at 100).
19. `w80h30-nocolor-7-member-1`: a toast message cuts with ASCII dots after a rule piece (`cargo-liner: ✨ clean ──...`).
20. `quota80-*-select-*` and `quota80-*-1-loading*`: the Ahead/Behind value cuts with 7 free cells to its right (`↑3 ↓35…       │`).
21. `w80h45-*-1-lint-runs`, `w80h45-*-2-*` and `w160h45-*-1-lint-runs`, `-6-lint-runs`: a detail pane pages (`1 of 2`) while a pane in the same column has 10 or more empty rows.
22. `quota80-*-loading-a/b/c`: the Git pane draws 7 blank rows with no state text while it loads.
23. `w160h45-*-2-ci-all`, `-2-main-checkout` and `-2-main-git-*`: the last Workspace Tests row is a number with no label (`7737`).

**Decision:** showrunner, 2026-11-09: all three go in one follow-up phase. Text on the border breaks the one-cell margin, so it is a defect and not a new look. Director, 2026-11-09: items 4 and 5 move here under the landing rule, because both are older than Phase 4v and outside its Spec. Showrunner, 2026-11-09: item 6 moves here from Phase 4v. Director, 2026-11-09: items 7 to 16 move here under the hard landing, after a compare with frames from the build before Phase 4v. Items 7 to 13, 15 and 16 are older than Phase 4v. Item 14 is older at 80 columns. At 60 columns with Lint on, the phase's kept list columns take 2 cells from Git, and a 10-cell date cannot fit in 9, so a short-date choice goes to the showrunner. Director, 2026-11-10: items 17 to 23 move here under the three-round cap. Phase 4v ran 3 repair rounds, and the branch is equal to or better than main on every live count. Items 18 to 23 are older than Phase 4v. Item 17 shows at 80 columns because the phase gave Package 16 cells, and the older Path rule then narrows the label. The check also listed the Package Lint value at 160 columns, which Spec 10 holds.

**Constraints:**
- Never cut a label. Cut a value, a name or a message first.
- Change no other look.

**Spec:**
1. **CI job header.** At 160 columns each CI job header draws whole, and the cells come from the Commit column (`tui/panes/ci/render.rs`).
2. **Table margin.** Lint Runs at every width and CI runs at 60 columns keep one cell between the left border and the text (`tui/panes/lints/render.rs`, `tui/panes/ci/render.rs`).
3. **Sccache hint row.** At 80 columns the hint row keeps one cell before the right border (`tui/sccache/render.rs`).
4. **Git dividers.** The Git pane draws one divider above the `Remotes` title, and a page that starts with a section draws no rule directly under the top border (`tui/panes/git/render.rs`).
5. **Commit cut.** A commit value does not leave a separator before `…` (`tui/panes/ci/render.rs`).
6. **Lint-on labels.** At 60 and 80 columns with Lint on, each column label sits over its column. The unit director sends the layout choice to the showrunner before any code (`tui/columns/`).
7. **Description height.** At 80 columns and 45 rows, the Git description is capped as at 30 rows, and Package does not pad to it (`tui/panes/git/render.rs`, `tui/panes/package/render.rs`).
8. **Group count.** A group row drops its count when the name would otherwise cut to one letter (`tui/panes/project_list/`).
9. **Disk rows.** `target/` and its value draw on one row when they fit, with one indent (`tui/panes/package/render.rs`).
10. **Lint Runs table.** The date column has a `Date` header at 160 columns, and the Package Lint value skips the empty icon slot (`tui/panes/lints/render.rs`, `tui/panes/package/render.rs`).
11. **Title spare cell.** A pane title with one spare cell draws balanced or in the no-side-space form. The unit director sends the choice to the showrunner before any code.
12. **Git dates.** A date value that does not fit drops the time and draws the whole date. The unit director sends the 9-cell form to the showrunner before any code (`tui/panes/git/render.rs`).
13. **Messages.** A Git message wraps onto a free row instead of cutting (`tui/panes/git/render.rs`).
14. **Wide names.** At 160 columns Languages and Targets names draw whole, and the cells come from free space in the project list (`tui/panes/layout.rs`).
15. **Path label.** A Path value that cuts keeps the label column of the other one-row fields (`tui/panes/package/render.rs`).
16. **Distinct names.** Cut member names stay different from each other. The unit director sends the cut form to the showrunner before any code (`tui/panes/project_list/`).
17. **Toast cut.** A toast message cuts with `…` and never after a rule piece (`crates/tui_pane/src/toasts/`).
18. **Ahead/Behind.** The value uses the free cells of its row before it cuts (`tui/panes/git/render.rs`).
19. **Free rows.** A detail pane takes the free rows of its column before it pages (`tui/panes/layout.rs`).
20. **Git loading.** The Git pane draws `loading…` until its data arrives (`tui/panes/git/render.rs`).
21. **Tests total.** The Workspace Tests sum row has the `Total` label (`tui/panes/package/render.rs`).
22. **Changelog.** Add the cargo-port and tui_pane entries.

**Files:**
- `crates/cargo-port/src/tui/panes/ci/` — Specs 1, 2 and 5
- `crates/cargo-port/src/tui/panes/lints/` — Specs 2 and 10
- `crates/cargo-port/src/tui/sccache/` — Spec 3
- `crates/cargo-port/src/tui/panes/git/` — Specs 4, 7, 12, 13, 18 and 20
- `crates/cargo-port/src/tui/columns/` — Spec 6
- `crates/cargo-port/src/tui/panes/package/` — Specs 7, 9, 10, 15 and 21
- `crates/cargo-port/src/tui/panes/project_list/` — Specs 8 and 16
- `crates/cargo-port/src/tui/panes/layout.rs` — Specs 14 and 19
- `crates/tui_pane/src/toasts/` — Spec 17
- the pane title renderer — Spec 11
- `crates/cargo-port/CHANGELOG.md` and `crates/tui_pane/CHANGELOG.md` — Spec 22

**Workers:** 1 writer: Specs 1-22 and their tests.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-port` and `verify.sh lint cargo-port` clean; no test takes a second. Drawn tests pin each defect at the width that showed it. Live captures at 60, 80 and 160 columns in colour and under NO_COLOR, judged by the design check. The checkpoint carries a `Change-Review:` line.

### Phase 5 — A CPU sample costs only the CPU pane  · status: dropped (user, 2026-11-06: "4 is not an issue that i care about")

**Depends on:** Phase 3b (both edit `tui/terminal/event_loop.rs`) and Phase 4b (it changes CPU drawing and pane geometry, and this phase measures after it)

#### Work Order

**Goal:** a CPU pane sample that changes nothing else does not render every pane. Idle CPU on the branch binary is not above main's.

**Source:** phase 3 review (event_loop.rs ≈285): on a CPU-only wake, the frame check allocates a backend, renders the whole UI to a candidate buffer and clones it. Phase 3's release idle measure (3 pairs, one scratch project): main 0.217/0.183/0.217% of one core, branch 0.267/0.250/0.250%. The median change (+0.033 points) is inside main's spread (0.034), so the cause is not proven. Astra's advice: `/tmp/claude/astra/20261102T204624Z-1292297/review.md`.

**Constraints:**
- Phase 3b removed the per-frame candidate render, allocation and copy that the source above names, and its as-built records equal idle pairs. Measure again after Phase 4b: 3 idle pairs, main's release build against the branch's, and one CPU-only wake on the branch (render calls, time).
- Build the partial render only when those 3 pairs show branch idle CPU above main's beyond the spread. Otherwise the phase completes with no code change: record the pairs in its As-built and close it.
- Full and CPU-only renders share one render setup in `tui/render.rs` (`render_tiled_panes` ≈144 and its render inputs). The event loop does not copy the render-context or pane-chrome rules.
- Both renders end in `prepare_terminal_buffer` (`tui/render.rs` ≈613), which applies the `NO_COLOR` reset and the VS16 cursor anchors. A partial render takes the same final step.
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
