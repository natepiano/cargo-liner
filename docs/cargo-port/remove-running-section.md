# Remove the Targets pane's Running section

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Deletes the Running section of cargo-port's Targets pane, its `K` kill binding, the 1 s process scan that feeds it, and the code only that scan used.

> **Production: cargo-port-cleanup** — unit `cleanup-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Delegation Context

- **Project:** cargo-port (`crates/cargo-port`), a TUI for inspecting and managing Rust projects.
- **Stack:** Rust 2024, ratatui through the workspace's `tui_pane` framework.
- **Layout:** every path below is under `crates/cargo-port/` unless it starts with `docs/`.
  - `src/tui/panes/targets/`: the Targets pane (table plus the Running box).
  - `src/tui/running_targets/`, `src/tui/process_refresh.rs`, `src/tui/process_observation/`: the process scan.
  - `src/tui/workspace_index.rs`, `src/project/cargo/workspace_index.rs`: the Cargo workspace index, whose only caller is the scan.
- **Key files** (line numbers from main at 614608dd):
  - `src/tui/panes/targets/running_subpane.rs`: draws the Running section. Delete it.
  - `src/tui/panes/targets/render.rs`: the pane's layout; :57-66, :74, :100-119, :128-157, :169-235, :296-305, :314-335; tests :570-634.
  - `src/tui/panes/targets/constants.rs`, `pane.rs`, `mod.rs`, `data.rs` (:157-159 stale doc comment).
  - `src/tui/panes/mod.rs`, `src/tui/panes/actions.rs`, `src/tui/panes/system.rs`.
  - `src/tui/keymap/actions.rs`, `src/tui/keymap/resolved.rs`, `src/tui/keymap/load.rs`, `src/tui/integration/framework_keymap/targets_pane.rs`, `tests/assets/default-keymap.toml`.
  - `src/tui/app/mod.rs`, `src/tui/app/construct.rs`, `src/tui/app/confirm_action.rs`, `src/tui/app/async_tasks/dispatch.rs`.
  - `src/tui/render.rs`, `src/tui/render_context.rs`, `src/tui/app_render_state.rs`, `src/tui/input/dispatch.rs`, `src/tui/interaction.rs`.
  - `src/tui/event_loop.rs`, `src/tui/background.rs`, `src/tui/startup_services.rs`, `src/tui/test_support.rs`.
  - `src/tui/running_targets/` (5 files), `src/tui/process_refresh.rs`.
  - `src/tui/process_observation/`: `executor.rs`, `snapshot.rs` and `mod.rs` go. `identity.rs` stays, trimmed.
  - `src/tui/workspace_index.rs`, `src/project/cargo/workspace_index.rs`, `src/project/cargo/workspace_index_api_tests.rs`, `src/project/cargo/mod.rs`, `src/project/mod.rs`, `src/project/cargo/metadata_store.rs`.
  - `src/tui/project_list/list.rs`, `src/tui/project_list_state.rs`, `src/constants.rs` (:66 `DOT_CARGO_DIR`).
  - `README.md`, `CHANGELOG.md`, `assets/pane-targets-numbered.png`.
  - `docs/cargo-port/as-built/targets-running-subpane.md`, `docs/cargo-port/as-built/running-process-tree.md`, `docs/cargo-port/tooltip.md` (:597).
- **Test lanes:** cargo-port: `crates/cargo-port/tests/`. This phase deletes code, so no new tests are needed. Both seats open as writers.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  - Nothing outside the Running section consumes the process scan, so it all goes. The following keep their own data and stay (from the 2026-10-06 code map):
    - the CPU pane (`tui_pane::CpuMonitor`);
    - the Output pane, which still shows a target launched from the Targets pane, and `Esc` still stops that run;
    - inflight and owned-run termination (`src/tui/state/inflight.rs`, `src/tui/state/owned_run_process_actor.rs`, `src/tui/terminal/processes.rs`);
    - the lint runtime and `running_toasts.rs`;
    - the project list's running indicator, which is lint-only (`project_list/list.rs:72`).
  - Every dependency in `crates/cargo-port/Cargo.toml` keeps another user:
    - `sysinfo`: `scan/disk_usage.rs`, and the Windows owned-run kill;
    - `libc`: the macOS process-start query;
    - `processkit`: `identity.rs`;
    - `sha2`: `metadata_store.rs`, `lint/paths.rs`.
  - No dead code is left: no `#[allow(dead_code)]`, and no item whose only caller was deleted. New `#[allow]`s need the user's review (user rule).
  - `crates/tui_pane` is unchanged. Its `Size::cap`, `.footer()` and `.lines()` APIs have other users. "Running box" there is only example wording. That scope note is the showrunner's.

## Phases

### Phase 1 — Remove the Running section and the process scan  · status: todo

#### Work Order

**Goal:** the Targets pane shows only its targets table, filling the pane. There is no Running section, no `K` kill binding, no 1 s process scan, and none of the code only they used. The CHANGELOG points readers to cargo-tile, installed from GitHub.

**Spec:**
1. **Targets pane UI.**
   - Delete `running_subpane.rs`.
   - In `render.rs`, remove the `build_running_rows` call (:57-66), the cursor PID reset (:74) and the capped Running box in `targets_region` (:100-119). Also remove `sync_running_cursor` (:128-157), the list build and height math and the `render_running_subpane` call (:169-235), and the footer code that exists only for a box below the table (:296-305, :314-335). The table takes the whole pane body.
   - Delete the Running-box layout tests (:570-634).
   - In `constants.rs`, remove `MIN_TABLE_ROWS`, `RUNNING_BOX`, `RUNNING_CAP_PERCENT`, `RUNNING_CHROME` and `TABLE_FOOTER`, plus the CPU/MEM/PID/PROFILE/OUTLINE/`TARGET_COL_MAX` widths (:32-50). Keep `TABLE_BOX` and `TABLE_CHROME`.
   - In `pane.rs`, remove `running_cursor_pid`, `cargo_group`, `expanded_parents` and their accessors (:26-42, :55-57, :67-93).
   - In `targets/mod.rs`, fix the module doc (:1-9) and drop the re-exports (:15, :27-33).
   - Fix the "running-first pre-pass" doc comment in `data.rs:157-159`.
2. **Actions, keys, confirm, input.**
   - `panes/mod.rs`: drop the re-exports (:119, :121, :127-128, :130-132), the import (:153) and the facades `execute_target_kill` (:204-209), `sync_running_targets_cursor` (:211) and `toggle_targets_tree_row` (:232-234).
   - `panes/actions.rs`:
     - remove the kill path, the cargo-group and outline toggles, `execute_target_kill` and `sync_running_cursor_pid` (:258-511);
     - remove the `Kill` dispatch (:254), the Left/Right expand and collapse in `navigate_targets` (:679-692) and the Enter toggle in `handle_detail_enter` (:913-917);
     - clean the imports (:14, :22-29, :55-56).
   - Remove the `Kill` action:
     - `keymap/actions.rs:32` and `keymap/resolved.rs:74`;
     - in `integration/framework_keymap/targets_pane.rs` :34, :40 and :59-87, remove `targets_kill_visibility` and simplify `targets_run_visibility`, which no longer needs its running-row case;
     - the golden `tests/assets/default-keymap.toml:53` and the fixture in `keymap/load.rs:809`.
     A user keymap that still lists `kill` gets only the existing "unknown entry" warning (`ignore_unknown_entries`), so it needs no migration.
   - Kill confirmation: remove `KillTarget` (`app/confirm_action.rs:2`, :16-24), `request_kill_confirm` (`app/mod.rs:807-823`, :1043), its render path (`render.rs:376-395`, :447) and its input path (`input/dispatch.rs:610-615`).
   - Mouse: remove the Running click re-anchor and tree toggle (`interaction.rs:51-58`) and the wheel path (`input/dispatch.rs:694-698`).
   - Reachability: in `app/mod.rs:1325-1332`, drop `|| running_targets.snapshot().has_instances()`. The pane is tabbable exactly when the project has targets.
   - Render context: remove the running-targets field and its plumbing (`render_context.rs:7`, :27; `app_render_state.rs:14`, :70; `app/mod.rs:674`, :700, :724; `render.rs:575`).
   - `app/mod.rs` tests: remove the kill label row (:3005-3012), the kill/run visibility tests (:3253-3315), their imports (:4751-4754) and the three outline/cargo-group tests (:6280-6420). Fix the test `PaneRenderCtx` (:15876).
3. **The process scan.**
   - Delete `src/tui/running_targets/` (all 5 files) and `src/tui/process_refresh.rs`.
   - Remove `RunningTargetsState` from `Panes` (`panes/system.rs:23`, :43-44, :75).
   - `event_loop.rs`: remove the refresh tick and its `FrameMetrics` and perf-log fields (:24-25, :30, :105-107, :134-135, :176-180, :185-205, :304-327) and the tests around :362/:376. The loop no longer wakes every 1 s for this.
   - Remove the wiring in `background.rs` (:28-30, :84-95), `app/construct.rs` (:43-45, :62, :274, :301-303, :336-354) and `app/mod.rs` (:195, :264-272: `process_refresh_executor` and the test-only `running_target_attribution_collection_count`).
   - Remove the startup gate `RunningTargetsPolling` from `startup_services.rs` (:67, :88, :107, :376-378, :416-418, :510, :531, :661, :681, :754, :776) and `test_support.rs:410`.
   - `process_observation/`:
     - delete `executor.rs`, `snapshot.rs` and what `mod.rs` holds beyond what `identity.rs` needs;
     - keep `identity.rs`, trimmed to what the owned-run path uses: `ProcessIdentity`, `observe_current_process_identity` and `revalidate_strong_process_identity`. Its callers are `tui/state/inflight.rs:23`, `tui/state/owned_run_process_actor.rs:43-47` and `tui/terminal/processes.rs:16-17`;
     - drop what only the snapshot used: the parent field on `PlatformProcessObservation::observe`, `ProcessIncarnation`, `ProcessFingerprint`, the creation-order evidence if nothing else reads it, and the `snapshot::{ProcessFieldObservation, ProcessFieldUnavailable, ReportedParent}` import. Let the compiler find the exact cut.
4. **Code only the scan used.**
   - Delete `src/tui/workspace_index.rs`, `src/project/cargo/workspace_index.rs` and `src/project/cargo/workspace_index_api_tests.rs`. Remove the `cargo_workspace_index` field (`app/mod.rs:198`, :264; `construct.rs:46`, :267-270, :300) and the re-exports (`project/cargo/mod.rs:9-11`, :37-51, and `project/mod.rs`).
   - Remove `ProjectListRevision`, `revision()` and `advance_revision` (`tui/project_list/list.rs:45`, :969, :1023, :1028, :1118, :1182; `project_list_state.rs:14`, :56), and the test at `app/async_tasks/dispatch.rs:335-347` that only checks them.
   - Remove `AcceptedCargoMetadataRevision` (`project/cargo/metadata_store.rs:32-49`, :115, :132).
   - Remove `DOT_CARGO_DIR` (`src/constants.rs:66`).
5. **Docs.**
   - Delete `docs/cargo-port/as-built/targets-running-subpane.md` and `docs/cargo-port/as-built/running-process-tree.md`.
   - Remove the Running subpane mention at `docs/cargo-port/tooltip.md:597`. Leave `docs/cargo-port/fix-SIGABRT.md` alone: it is a historical plan.
   - `README.md`:
     - In the Features list (:18), drop "and running-target markers".
     - In the Targets pane list, delete items 4 and 5 (the second subpane and the `K` kill). Then add one line after item 3: to see every running cargo process on the machine, use cargo-tile, installed from GitHub for now with `cargo install --git https://github.com/natepiano/cargo-liner cargo-tile`.
   - `assets/pane-targets-numbered.png` still shows callouts 4 and 5. The annotations were made by hand, and no script in the repo makes them. If the Running box and both callouts sit entirely below the table, crop them off with `magick` so the pane stays whole with its bottom border; otherwise leave the image and name it in the checkpoint notice for the user to recapture.
6. **CHANGELOG** (`crates/cargo-port/CHANGELOG.md`, `## [Unreleased]` → `### Removed`). Follow `/changelog`'s format rules: one bullet, one line. The user asked that this entry tell readers to install cargo-tile, from GitHub for now, to see every running cargo process. Add exactly:

   ```markdown
   - **Breaking:** Remove the Running section of the Targets pane, its `K` kill binding, and the process scan behind them. To see every running cargo process, install [cargo-tile](https://github.com/natepiano/cargo-liner/tree/main/crates/cargo-tile) from GitHub for now: `cargo install --git https://github.com/natepiano/cargo-liner cargo-tile`.
   ```

   In the existing build-monitor bullet in the same section, replace `Use [cargo-tile](https://crates.io/crates/cargo-tile) instead.` with `Use [cargo-tile](https://github.com/natepiano/cargo-liner/tree/main/crates/cargo-tile) instead, installed from GitHub for now.` crates.io holds cargo-tile 0.1.0, far behind the workspace's 0.2.78-dev, so both entries in this release point to the same install source. That edit is the showrunner's call.

**Files:**
- `crates/cargo-port/src/tui/panes/targets/running_subpane.rs` — delete
- `crates/cargo-port/src/tui/panes/targets/render.rs` — remove the Running box, its layout and tests
- `crates/cargo-port/src/tui/panes/targets/constants.rs` — remove Running constants and column widths
- `crates/cargo-port/src/tui/panes/targets/pane.rs` — remove Running cursor and tree state
- `crates/cargo-port/src/tui/panes/targets/mod.rs` — doc and re-exports
- `crates/cargo-port/src/tui/panes/targets/data.rs` — stale doc comment
- `crates/cargo-port/src/tui/panes/mod.rs` — re-exports and facades
- `crates/cargo-port/src/tui/panes/actions.rs` — kill, toggles, navigation
- `crates/cargo-port/src/tui/panes/system.rs` — drop `RunningTargetsState`
- `crates/cargo-port/src/tui/keymap/actions.rs` — drop `Kill`
- `crates/cargo-port/src/tui/keymap/resolved.rs` — drop `Kill`
- `crates/cargo-port/src/tui/keymap/load.rs` — fixture
- `crates/cargo-port/src/tui/integration/framework_keymap/targets_pane.rs` — kill visibility
- `crates/cargo-port/tests/assets/default-keymap.toml` — golden keymap
- `crates/cargo-port/src/tui/app/mod.rs` — kill confirm, reachability, fields, tests
- `crates/cargo-port/src/tui/app/construct.rs` — scan and index wiring
- `crates/cargo-port/src/tui/app/confirm_action.rs` — drop `KillTarget`
- `crates/cargo-port/src/tui/app/async_tasks/dispatch.rs` — revision test
- `crates/cargo-port/src/tui/render.rs` — kill confirm render, render context
- `crates/cargo-port/src/tui/render_context.rs` — running-targets field
- `crates/cargo-port/src/tui/app_render_state.rs` — running-targets field
- `crates/cargo-port/src/tui/input/dispatch.rs` — kill confirm input, wheel
- `crates/cargo-port/src/tui/interaction.rs` — Running click handling
- `crates/cargo-port/src/tui/event_loop.rs` — refresh tick and metrics
- `crates/cargo-port/src/tui/background.rs` — refresh wiring
- `crates/cargo-port/src/tui/startup_services.rs` — `RunningTargetsPolling` gate
- `crates/cargo-port/src/tui/test_support.rs` — gate reference
- `crates/cargo-port/src/tui/mod.rs` — module declarations
- `crates/cargo-port/src/tui/running_targets/` — delete the directory
- `crates/cargo-port/src/tui/process_refresh.rs` — delete
- `crates/cargo-port/src/tui/process_observation/executor.rs` — delete
- `crates/cargo-port/src/tui/process_observation/snapshot.rs` — delete
- `crates/cargo-port/src/tui/process_observation/mod.rs` — reduce to what `identity.rs` needs
- `crates/cargo-port/src/tui/process_observation/identity.rs` — trim to the owned-run path
- `crates/cargo-port/src/tui/workspace_index.rs` — delete
- `crates/cargo-port/src/project/cargo/workspace_index.rs` — delete
- `crates/cargo-port/src/project/cargo/workspace_index_api_tests.rs` — delete
- `crates/cargo-port/src/project/cargo/mod.rs` — re-exports
- `crates/cargo-port/src/project/mod.rs` — re-exports
- `crates/cargo-port/src/project/cargo/metadata_store.rs` — drop `AcceptedCargoMetadataRevision`
- `crates/cargo-port/src/tui/project_list/list.rs` — drop `ProjectListRevision`
- `crates/cargo-port/src/tui/project_list_state.rs` — drop revision plumbing
- `crates/cargo-port/src/constants.rs` — drop `DOT_CARGO_DIR`
- `crates/cargo-port/README.md` — Features line, Targets pane items, cargo-tile pointer
- `crates/cargo-port/assets/pane-targets-numbered.png` — crop, or leave for the user
- `crates/cargo-port/CHANGELOG.md` — Removed entry, build-monitor link
- `docs/cargo-port/as-built/targets-running-subpane.md` — delete
- `docs/cargo-port/as-built/running-process-tree.md` — delete
- `docs/cargo-port/tooltip.md` — Running subpane mention

**Seats:** 2 writers. The work splits UI from data source.
- `impl` — UI, keys and docs: Spec items 1, 2, 5 and 6, which cover `src/tui/panes/**` except `system.rs`, `src/tui/keymap/**`, `src/tui/integration/**`, `src/tui/app/confirm_action.rs`, `src/tui/render.rs`, `src/tui/render_context.rs`, `src/tui/app_render_state.rs`, `src/tui/input/**`, `src/tui/interaction.rs`, `tests/assets/default-keymap.toml`, the README, the PNG, the CHANGELOG and the docs. It also owns hub `src/tui/app/mod.rs`, which both items touch; the other writer messages it for the field and constructor lines.
- `test` — opens as impl. Process scan and the code only it used: Spec items 3 and 4, which cover `src/tui/running_targets/`, `src/tui/process_refresh.rs`, `src/tui/process_observation/**`, `src/tui/event_loop.rs`, `src/tui/background.rs`, `src/tui/app/construct.rs`, `src/tui/startup_services.rs`, `src/tui/test_support.rs`, `src/tui/panes/system.rs`, the workspace index files, `src/project/**`, `src/tui/project_list/list.rs`, `src/tui/project_list_state.rs`, `src/tui/app/async_tasks/dispatch.rs` and `src/constants.rs`. It also owns hub `src/tui/mod.rs`, for the module declarations.

**Constraints from prior phases:** none.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh check cargo-port` green.
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-port` green.
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` green, with no new `#[allow]`.
- `rg -n 'running_targets|RunningTargets|process_refresh|running_subpane|KillTarget|WorkspaceIndex|ProjectListRevision|AcceptedCargoMetadataRevision|DOT_CARGO_DIR' crates/cargo-port` finds nothing.
- `crates/cargo-port/Cargo.toml` dependencies are unchanged.
- Smoke: launch cargo-port in a real terminal, select a project with targets, launch an example with Enter. The Targets pane shows only the table, filling the pane. The Output pane shows the run, `Esc` stops it, and `K` does nothing in the Targets pane.
- The CHANGELOG `### Removed` section holds the new bullet verbatim and the edited build-monitor link.
