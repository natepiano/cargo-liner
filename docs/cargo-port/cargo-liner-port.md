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

### Phase 4f — Bold and reverse survive NO_COLOR in every tui_pane app  · status: todo

**Depends on:** Phase 4d

#### Work Order

**Goal:** under `NO_COLOR=1` in a real terminal, the focused title is bold and the selected row is reversed in `cargo-handler`, `cargo-tile` and the `cargo-berth` board, as they are in `cargo-port` today.

**Why:** handler found this live. Under `NO_COLOR`, crossterm writes `ESC[;m`, a full style reset, for each `SetColors` (`crossterm-0.29.0` `style/types/colored.rs:100`). `ratatui-crossterm` sets modifiers first and colours second (`ratatui-crossterm-0.1.2` `lib.rs:259`), so each reset clears bold and reverse. `cargo-port` does not show the defect: `prepare_terminal_buffer` sets each cell's `fg` and `bg` to `Reset`, so the backend writes no `SetColors`. The other apps have no such step.

**Constraints:** `cargo-port`'s drawn output does not change. In colour mode no app's output changes. Showrunner: this phase runs after 4d and before 4c, and the proof is the terminal bytes.

**Spec:**

1. **One step.** `tui_pane` owns the colour-clearing step that `prepare_terminal_buffer` holds today: under `ColorMode::NoColor`, each cell of the finished frame gets `fg` and `bg` `Reset`. Under `ColorMode::Colored` it changes nothing. `cargo-port`'s `prepare_terminal_buffer` calls it and keeps its VS16 work.
2. **Every app.** `cargo-handler`, `cargo-tile` and the `cargo-berth` board call the step at the end of each frame, with the mode that Phase 4d gives them.
3. **No global change.** Do not call `force_color_output`. Crossterm keeps its own suppression for any write outside the buffer.
4. **The byte test.** A `tui_pane` test draws a frame with colour, bold and reverse through the real `CrosstermBackend` into a byte buffer, in a child process with `NO_COLOR=1`. It requires `ESC[1m` and `ESC[7m` with no reset before the text they style, and no colour code. The test fails when the step is removed, and it takes under a second.

**Files:**
- `crates/tui_pane/src/` — the step and the byte test
- `crates/tui_pane/CHANGELOG.md` — the new public function
- `crates/cargo-port/src/tui/render/mod.rs` — calls the step
- `crates/cargo-handler/src/` — calls the step
- `crates/cargo-tile/src/` — calls the step
- `crates/cargo-berth/src/board/tui.rs` — calls the step

**Workers:** 1 writer. The step and its calls are one change.

**State:** none.

**Acceptance gate:** `verify.sh test tui_pane`, `verify.sh test cargo-port`, `verify.sh test cargo-handler`, `verify.sh test cargo-tile`, `verify.sh test cargo-berth` and `verify.sh lint tui_pane` pass. The director captures raw pty bytes of `cargo-port` and `cargo-handler` under `NO_COLOR=1`: each capture holds `ESC[1m` and `ESC[7m` with no reset before the text they style, no `ESC[;m` and no colour code.

### Phase 4c — cargo-port's remaining screen defects read clearly  · status: todo

**Depends on:** Phase 4b, Phase 4d (the Spec 20 popup border takes the colour mode value), Phase 4f

#### Work Order

**Goal:** the screen defects that the Phase 4b design check found outside the Phase 4b Work Order and that are wrong, clipped or unreadable are gone. Every other look stays as it is today.

**Source:** Phase 4b design checks, 2026-11-06, the Phase 2x architect review for Deleted targets, and the user's change reviews of Phase 4b for Projects columns. Most rows also show on main's shots at `3fb98ed6e`. Shots: `/tmp/claude/unit/b6e56837-8fc7-4cd5-ba71-0ff1c804cfd1/phases/4/live/shots_4b/` and `/tmp/claude/unit/b6e56837-8fc7-4cd5-ba71-0ff1c804cfd1/phases/4/live/shots_main/`.

**Constraints:**
- User, 2026-11-06, on the Languages icons: "i did not ask for a change in this". Showrunner, the same day: a look the user sees today stays unless it is wrong, clipped, unreadable or against a user rule. A dropped Spec below keeps its number.
- Phase 4b's results stay as they are, except where a Spec below changes them.
- Colours stay in the theme. Do not hard-code a new value at a draw site.
- Each item gets a red test that reads the drawn buffer, under one second.
- These existing assertions change. Record each one in an Assertion-Change line, and weaken no other assertion:
  - Spec 15: `narrow_ci_draws_only_complete_columns`, which requires `Branch` at width 24.
  - Spec 19: `sccache_reasons_explain_the_uncached_call`, `parse_stat_line_explains_raw_reason_keys`.
- **Constraints from prior phases:**
  - Phase 4d: drawing code takes `tui_pane::ColorMode` as a value. `ColorMode::from_environment()` is the only read of `NO_COLOR`, and each app holds the value in `Framework::color_mode`. Spec 20's border in `crates/tui_pane/src/pane/popup.rs` takes the value from its caller. Tests state their mode.
  - `theme_roles::paint_selection` applies the selection style last across the inner row width. A selected row keeps one fill and one text colour, floor 4.5:1 by `covered_contrast`.
  - Keep `narrow_layout_keeps_git_values_and_hides_unreadable_panes` and `narrow_layout_moves_focus_once_and_restores_panes_without_moving_focus` in `tui/render/readability_tests.rs`.
  - `tui_pane::selection_style` (`crates/tui_pane/src/pane/state.rs:112`) also serves cargo-tile and cargo-handler. Their behaviour stays, and so do the Active reverse video and the Remembered bold under `NO_COLOR`.
  - `PackageDeletion` already owns the Package pane's deleted title and Disk rows.

**Spec:**
1. **Deleted project panes.** Languages of a Deleted project does not show the last data as if it were current. One named presentation state says so, and the pane reads it. Targets of a Deleted package do not launch, and they launch again when the project comes back (user, 2026-11-06, via the showrunner: "correct a deleted project's targets do not launch"):
   - `handle_target_action` (`tui/panes/actions.rs:51`) starts a cached target with no deletion check, so enter starts a cargo run in a folder that is gone. Enter or a click on such a target starts no run.
   - Availability belongs to each target's owning package. A workspace selection combines every member's cached targets (`tui/panes/targets/data.rs:242`), so a check of the selected workspace's `Visibility` cannot tell a Deleted member from a live member.
   - The Targets pane draws no row for a target of a Deleted package, and the run hint hides when no row remains. A whole Deleted project draws an empty pane. A live workspace keeps the rows of its live members.
   - Tests cover deletion, recreation, dismissal and rescan, a live workspace with both Deleted and live target owners, and the data path through normal selection and finder selection.
2. **Right margin in every pane.** Dropped: today's look is not broken.
3. **One column word.** Dropped: today's look is not broken.
4. **One name for the disk cache.** Dropped: today's look is not broken.
5. **Counts and units whole at 80 columns.** The title `Targets: binary (…` keeps its count, and `23.8 G…` keeps its unit.
6. **Sections at 80 columns.** `├─ Structure ─┤` does not sit above `Path` and `Disk`. Section titles start in one column, and the divider draws its `┬` over the vertical line.
7. **Page marks.** A CPU pane that shows part of its bars shows a page mark, or hides the bar row. The mark stays in the pane body, where it is today.
8. **Status row gaps.** Dropped: today's look is not broken.
9. **One description.** `No description …` does not show beside a pane that shows the description.
10. **sccache popup words.** Dropped: today's look is not broken.
11. **Behind a popup.** A popup leaves no cut words or one-cell scraps behind it (`hQL`, `al`, `tem`, and `h`, `…`, `▶` at 80 columns): clear or dim all cells behind it.
12. **Hints.** `enter activate` shows only on a row that acts on enter. Every other hint word and key case stays.
13. **Languages colours.** Dropped: today's look is not broken.
14. **Disk colours.** Disk values keep today's colours. The scale comes from rows that are not Deleted, so a Deleted row's `0 B` moves no other row. A Deleted row keeps its `[x]` look.
15. **CI runs at 80 columns.** In branch mode, `Branch` hides first at narrow width, because the title names the branch (`ci_panel_title`, `tui/panes/ci/render.rs:619`). Its cells go to `Commit` and a whole status column. In all mode, the title names no branch and rows can show several branches, so `Branch` stays. Drawn tests cover both modes.
16. **One total word.** Dropped: today's look is not broken.
17. **sccache popup values.** Dropped: today's look is not broken.
18. **One active fill.** Dropped: today's look is not broken.
19. **The `-` reason.** The sccache reason row for `-` reads `Source read from standard input`. `No reason reported` (`tui/sccache/stats.rs:73`) is false: sccache records `-` when rustc reads its source from standard input.
20. **Popup focus under `NO_COLOR`.** An open popup draws the thick border of a focused pane. In colour its border shows focus; under `NO_COLOR` it now draws the thin border of an unfocused pane.
21. **Rows after the reasons.** Dropped: today's look is not broken.
22. **Git rates while loading.** Dropped: today's look is not broken.
23. **One title separator.** Dropped: today's look is not broken.
24. **Project list columns stay.** User, 2026-11-06, on the change reviews of Phase 4b's selected row and narrow windows: "you're getting rid of columns before you make project names have an ellipipsis - columns should always be preserved - the project names should get ellipsis first". The user's window is about 95 columns wide, with long worktree names such as `proof_crate [claude-build-followups-sdlc-merge-gate/scripts/aws_target/pr…`, and shows only `Projects` and `Disk`.
   - Cause: Phase 4b's `ProjectListWidths::fitted` (`tui/columns/project.rs:366`) sets whole columns to width 0 while the name keeps its full measured width. It cuts the name only after that.
   - The name gives up its width first and takes the `…`. Its floor keeps the whole `Projects` heading and the cells that the `Lint` heading borrows.
   - The Projects pane keeps half the window, as it does now. It takes more only when every column needs more with the name at its floor. `resolve_layout` and `top_pane_widths` (`tui/panes/layout.rs`) each hold `left_width.min(area.width / 2)`, and one function replaces both.
   - A column goes only when the window cannot draw it with the name at its floor. The order stays: `Main`, `Sync`, `Git`, `Type`, `CI`, `Lint`.
   - A row prefix wider than the name's floor does not push the row's other columns out of line.
   - Tests read the drawn buffer at widths 95 and 80, with short names and with the user's long worktree names: every heading is whole, each row's `Disk` value ends on one right edge, and each long name ends in `…`.
   - Showrunner, 2026-11-06: "at 80 columns, Targets takes CPU's place; CPU hides until there is room. Why: Targets is where the user acts (Enter runs a target); CPU is only a readout. Tab skips the hidden CPU pane as 4b's rule says." The middle row's priority is Targets, CPU, Languages. Targets needs fewer cells than CPU, so CPU never shows alone.

**Files:**
- `crates/cargo-port/src/tui/` — the panes, the status text, the sccache popup and their tests
- `crates/tui_pane/src/` — only when the cause of an item lives there
- `.config/nextest.toml` — tests do not read the shell's `NO_COLOR`; normal runs still do

**Workers:** 1 writer + 1 tester
- Writer: production files, shared render files, module declarations and tests inside production files.
- Tester: the per-pane `readability_tests.rs` files, `tui/render/readability_tests.rs`, `tui/sccache/readability_tests.rs` and `tui/terminal/no_color_tests.rs`.

**State:** target availability (available, or held because the owning package is Deleted), owned by the Targets pane, read from the owning package's `Visibility` at each draw and action.

**Acceptance gate:** `verify.sh test cargo-port`, `verify.sh test tui_pane` when it changes, and `verify.sh lint cargo-port` clean; no test takes a second. The director presses enter on a Deleted project's target in the live app, and no run starts. Live shots at 95 and 80 columns, with long worktree names, show every Projects column. The unit director takes live shots at 160x45 in colour, 160x45 under `NO_COLOR=1` and 80x30 in colour. A fresh design check on those shots raises none of these items. The Phase 4b `NO_COLOR` shots show single `#d4d4d4` pixels in blank cells, where the `.ansi` capture has only spaces; the director finds their cause in the live harness before those shots.

### Phase 4e — cargo-port's popups and remaining words read clearly  · status: todo

**Depends on:** Phase 4c

#### Work Order

**Goal:** the screen defects that the Phase 4c design check found outside the Phase 4c Work Order and that are wrong, clipped or unreadable are gone. Every other look stays as it is today.

**Source:** Phase 4c design check, 2026-11-06, moved here by the production's rule that a phase fixes only its own change. Shots: `/tmp/claude/unit/b6e56837-8fc7-4cd5-ba71-0ff1c804cfd1/phases/4/live/shots_4c/`.

**Constraints:**
- Phase 4b and 4c results stay as they are, except where a Spec below changes them.
- Colours stay in the theme. Do not hard-code a new value at a draw site.
- Each item gets a red test that reads the drawn buffer, under one second.
- User, 2026-11-06, on the Languages icons: "i did not ask for a change in this". Showrunner, the same day: a look the user sees today stays unless it is wrong, clipped, unreadable or against a user rule. A dropped Spec below keeps its number.
- Spec 8 changes what the user sees of a past choice. Send it to the showrunner with its buildable answers before the build.

**Spec:**
1. **Settings popup text.** Settings wraps to the real popup width at 80 columns, so no text is lost (`global`, `04cfd1/pha`). Paths keep one blank cell before the border and break at `/`.
2. **Settings selection.** A selected Settings row has one text colour on the fill, floor 4.5:1 by `covered_contrast` (`Invert scroll` measures 2.86:1 and `ON` 4.25:1).
3. **Languages names.** Names keep their meaning at the normal pane width: `└ integration`, `Python`, `Markdown`, `Autoconf`.
4. **Package values.** `🔴 local 7 / g…` shows both CI counts. `No CI workflo…` and `No description …` use a shorter text that fits.
5. **Startup toast.** The toast leaves no one-cell `│` scraps, no cut `┌ No lint runs ───` and no blank row across the panes.
6. **Narrow hints.** Dropped: today's look is not broken.
7. **CI columns.** The `Total` column shows a true total, or takes a name that says what it shows (`Other` reads `13m 59s` above `Total` `4m 16s`).
8. **Deleted Package values.** The deleted Package pane still shows `Kind`, `Version` and `Edition` in normal style, and `No description available`. Decide the look through the showrunner: path and deleted state only, or the old values dimmed.
9. **Package value colours.** Dropped: today's look is not broken.
10. **One hint spacing.** Dropped: today's look is not broken.
11. **sccache popup.** Dropped: today's look is not broken.
12. **Common words and one form.** Dropped: today's look is not broken.
13. **Popup structure.** Dropped: today's look is not broken.
14. **Selected-row inks.** Routed by the showrunner from cargo-handler's old Phase 25 Spec 10. At phase 18, cargo-port's selected-row inks on Default Dark measured under the floor of 4.5:1 by `covered_contrast`: CI circle 2.55:1, `Og` glyph 2.42:1, `Main` `☑` 2.10:1, disk text 4.25:1. Measure each on the tree. Fix only an ink that is still under the floor, and keep the look otherwise. If all pass, record `passes` and close this Spec.

**Files:**
- `crates/cargo-port/src/tui/` — the panes, the popups, the status text and their tests
- `crates/tui_pane/src/` — only when the cause of an item lives there
- `.config/nextest.toml` — tests do not read the shell's `NO_COLOR`; normal runs still do

**Workers:** 1 writer + 1 tester
- Writer: production files, shared render files, module declarations and tests inside production files.
- Tester: the per-pane `readability_tests.rs` files, `tui/render/readability_tests.rs` and `tui/sccache/readability_tests.rs`.

**State:** none.

**Acceptance gate:** `verify.sh test cargo-port`, `verify.sh test tui_pane` when it changes, and `verify.sh lint cargo-port` clean; no test takes a second. The unit director takes live shots at 160x45 in colour, 160x45 under `NO_COLOR=1` and 80x30 in colour. A fresh design check on those shots raises none of these items.

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
