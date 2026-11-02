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

**Source:** showrunner, 2026-11-02 10:40, a blocking cargo-mend bug (user rule: mend bugs come first). In cables-unit's lint of hana (installed mend 149827ba, `--no-cache`), the rustix 1.1.4 build script panicked at `build.rs:283:64`: `unwrap()` on `Err(Os { code: 32, BrokenPipe })`. Mend then reported "compiler failed while validating this crate". Log: `/tmp/claude/delegate/fc206247-76c0-4607-b268-025299592a62/phases/stall/evidence/mend-brokenpipe-1ce7.log`.

**Cause known so far:** rustix's `can_compile` spawns `$RUSTC_WRAPPER $RUSTC --crate-type=rlib --emit=metadata --target <t> -o <out> -` with a piped stdin, then writes the probe source to it. EPIPE means the wrapper process (or what it runs) exited before it read stdin. A direct run of the same wrapper copy, with and without `MEND_PASSTHROUGH_RUSTC_WRAPPER=sccache`, read stdin and exited 0. So the early exit needs the environment of the real mend run.

**Second symptom (showrunner, 10:35 run):** the libc 0.2.186 build script (build.rs:219) could not start stable rustc: "libLLVM.so.23.1-rust-1.99.1-stable: cannot open shared object file". The file exists, and rustc's RUNPATH is `$ORIGIN/../lib`. So under mend, the loader path has an entry ahead of RUNPATH that holds a `librustc_driver` without its libLLVM. The rustix broken pipe is the same fault: rustc died at load before it read stdin. Log: `/tmp/claude/delegate/fc206247-76c0-4607-b268-025299592a62/phases/cable-stall/lint-b109-cargo-mend.log`.

**Spec:**
1. Reproduce: find the invocation and environment in which the mend wrapper exits before it reads stdin. Record the cause.
   Fix both symptoms at the cause. Add a test that a build script can start stable rustc under mend.
2. Fix it in cargo-mend where the defect lives. A wrapper invocation that compiles from stdin reads its input, or hands stdin to the compiler, on every path.
3. A regression test, under one second, fails before the fix and passes after it.
4. CHANGELOG: one line in `crates/cargo-mend/CHANGELOG.md`.

**Files:** `crates/cargo-mend/src/compiler/` (driver, probes, wrapper setup), `crates/cargo-mend/src/main.rs` only if the cause is there, the tests next to the changed code, `crates/cargo-mend/CHANGELOG.md`.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean; the regression test fails on 149827ba and passes on the branch.

### Phase 2q — tui_pane owns the expand/collapse model for cargo-port and cargo-handler  · status: todo

**Depends on:** none

#### Work Order

**Goal:** cargo-port's expand/collapse model and its keys move into `tui_pane` as one copy. cargo-port's project list uses it and behaves exactly as today. cargo-handler Phase 45 then uses the same model for its parent/child rows.

**Source:** the user approved this phase through the showrunner, 2026-11-02. The user's words in handler, 2026-10-29: "rows should expand and collapse the way they do in cargo ports project list - in handler where wehave parent child relationships drawn - by default we should show just the first level of expansion". handler-unit gave its row shape on 2026-11-02 (below).

**State model (write and keep this first):**
- **Expand key:** a value of the app's key type `K` (`Eq + Hash + Clone`) that names one row that can have children. cargo-port: today's `ExpandKey` (`crates/cargo-port/src/tui/project_list/visible_rows.rs:22`, index-based). cargo-handler: `RowIdentity` (machine, pid) in `cargo-handler` `census/placement.rs`.
- **Default rule:** a function of the row's depth that says open or closed. cargo-port: closed at every depth. cargo-handler: depth 0 open, deeper rows closed.
- **Open rows:** a row is open when its override says so, else when its default says so. The model holds one record: the overrides, `K` to open or closed. No second record holds the same fact.
- **Operations:** expand (override to open), collapse (override to closed), toggle (flip the effective state), expand-all (every expandable row open), collapse-all (every expandable row closed), reset (drop every override), retain (drop the overrides of keys no longer in the tree, after a rebuild).
- **Collapse on a closed or leaf row** moves the selection to the row's parent and collapses the parent, as the project list does today. The app supplies the parent of a row through a trait; the model does not know the app's row kinds.
- **Keys and actions:** the expand, collapse, expand-all and collapse-all actions and their default key bindings live in `tui_pane`, so both apps use the same keys.

**Constraints:**
- One copy: after the move, cargo-port has no second expand state or second implementation of an operation.
- cargo-port behaves exactly as today: the same rows open after every key, the same selection after collapse, the same rows after a rescan. Its existing tests pass unchanged, with no weaker check.
- Public in `tui_pane` only what cargo-port and cargo-handler call. Keep the rest `pub(crate)` or private.
- cargo-port's row kinds (project, group, member, worktree, vendored) stay in cargo-port behind the trait.
- No test takes a second.

**Spec:**
1. The model type in `tui_pane` with the operations above, generic over `K`, with the default rule as a value given at construction.
2. The tree trait that an app implements: expandable rows with key and depth, and the parent of a row.
3. The actions and default key bindings in `tui_pane`; cargo-port's project list binds them.
4. cargo-port's project list uses the model: `ProjectListState.expanded`, `selection.rs`, the expand and collapse code in `list.rs`, `app/navigation/expand.rs`, and `visible_rows.rs` read it.
5. Unit tests in `tui_pane` for each operation and for both default rules (all closed; depth 0 open), with string or integer keys.
6. CHANGELOG lines in `crates/tui_pane` and `crates/cargo-port` if they keep one.

**Files:**
- `crates/tui_pane/src/` — the new module (writer names it) and its `lib.rs` export; the keymap actions
- `crates/cargo-port/src/tui/project_list/` — `selection.rs`, `list.rs`, `visible_rows.rs`, `mod.rs`
- `crates/cargo-port/src/tui/project_list_state.rs`, `crates/cargo-port/src/tui/app/navigation/expand.rs`, `crates/cargo-port/src/tui/interaction.rs`
- cargo-port keymap and action files that bind expand and collapse (writer lists them)
- approved during the phase: `crates/cargo-port/src/tui/panes/project_list/{tree_rows.rs,tree_render.rs}`, `crates/cargo-port/src/tui/app/async_tasks/{tree.rs,dispatch.rs}`, `crates/cargo-port/src/tui/app/mod.rs` (test type-only conversions), `crates/cargo-port/src/tui/keymap/`, `crates/cargo-port/src/tui/integration/framework_keymap/project_list_pane.rs`, both CHANGELOGs

**Workers:** 1 writer (impl) owns the code and the tests.

**Acceptance gate:** `verify.sh test tui_pane`, `verify.sh test cargo-port` and `verify.sh lint` clean on a gate tree; cargo-port's existing expand and collapse tests unchanged; no test takes a second. The notice carries `Change-Review: none` because nothing visible changes.

### Phase 2s — mend --fix keeps the cfg of an import it moves  · status: todo

**Source:** showrunner, 2026-11-02 10:55. A cargo-mend bug that does not block other work. It runs beside 2q after 2r lands; the files do not overlap.

**Defect:** `cargo mend --fix` on cargo-handler moved eight `use libproc::…` imports out of a `#[cfg(target_os = "macos")]` function to module scope (cargo-handler `census/codex.rs:51-58`). The moved imports lost the cfg, so Linux failed with eight E0433 errors, and mend rolled back. Record with the code before and after: `/tmp/claude/unit/be6481c9-047e-4add-83c1-870d55a33343/phases/60a/evidence/mend-e0433.md`.

**Spec:**
1. When mend moves an import out of an item, the import keeps every cfg of that item. If it cannot keep them, mend leaves the import where it is.
2. A fixture with a cfg-gated function and a cfg-only dependency. Its test fails on the current code and passes after the fix, in under one second where a unit test can show it.
3. CHANGELOG: one line in `crates/cargo-mend/CHANGELOG.md`.

**Files:** the cargo-mend code that moves imports to module scope, and its tests, a new fixture under the cargo-mend test fixtures, `crates/cargo-mend/CHANGELOG.md`.

**Acceptance gate:** `verify.sh test cargo-mend` and `verify.sh lint cargo-mend` clean.

### Phase 3 — cargo-port draws a frame only when something changed  · status: todo

**Depends on:** Phase 2b

#### Work Order

**Goal:** an idle cargo-port uses under 1% of one core and draws no frame when nothing visible changed. During builds it stays under 3% of one core and under 2 frames each second.

**Source:** showrunner, 2026-10-28; the user ranks CPU work above all other work. Measured on natedev at 11:12–11:25: the cargo-port main thread used 14% of one core; perf put 90% of it in `Path == Path` compares from `RootItem::at_path` (`crates/cargo-port/src/project/root_item.rs`), about 4 frames each second at about 30 ms each. Notes: `/tmp/claude-1000/-home-natepiano-rust-cargo-liner/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/scratchpad/cpu_investigation.md`, section "cargo-port: a full frame".

**Constraints:**
- From phase 2b: a `DiskUsage` reply can change a row from `Visible` to `Deleted` with the byte count still zero (`async_tasks/disk_handlers.rs` `apply_disk_usage` ≈65), so equal bytes do not mean equal visible state. Test that this change draws once and an identical later reply draws nothing.
- From phase 2d: the row error state for a manifest cargo rejects is a visible input. Test redraws for an unselected row's error, its recovery, and `ProjectDiscovered`.
- The detail cache owns `Option<DetailCacheKey>` (`tui/panes/data.rs` ≈21). Replace it with a named state that tells cleared details from details current for a key, like the new caches. The APIs that read or write it (`detail_is_current`, `set_detail_stamp`, `clear_detail_data` in `tui/panes/data.rs`) and their tests take and return that state too, so no caller reads a bare absence.
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
- `crates/cargo-port/src/tui/terminal/event_loop_tests.rs` — new
- `crates/cargo-port/src/tui/project_list/path_index_tests.rs` — new
- `crates/cargo-port/CHANGELOG.md`

**Workers:** 1 writer (impl4) owns the implementation and every test file; the unit director owns the live CPU measurements (spec 5).
- `impl` — specs 1–4 and 6, the frame counter, the two slow interaction tests in `tui/app/mod.rs`, and the tests in `tui/terminal/event_loop_tests.rs` and `tui/project_list/path_index_tests.rs`; hub: `crates/cargo-port/src/tui/project_list/list.rs`.

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
- `crates/cargo-port/src/tui/app/async_tasks/tree.rs` — rescan keeps Deleted entries (entry point `prepare_rescan`)
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
