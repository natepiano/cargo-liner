# Remove the Targets pane's Running section

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Deletes the Running section of cargo-port's Targets pane, its `K` kill binding, the 1 s process scan that feeds it, and the code only that scan used.

> **Production: cargo-port-cleanup** — unit `cleanup-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Delegation Context

- **Project:** cargo-port (`crates/cargo-port`), a TUI for inspecting and managing Rust projects.
- **Project started:** 2026-10-06T19:42:58.007+00:00
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
  - `src/tui/project_list/list.rs`, `src/tui/project_list_state.rs`.
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

### Phase 1 — Remove the Running section and the process scan  · status: done

#### As-built

- The Targets pane shows only the targets table, which fills the pane body: `render.rs` places a single `TABLE_BOX`, and `build_targets_title(focus, cursor: usize, data)` takes a plain cursor. There is no Running subpane, no `Kill` action or `K` binding, no `KillTarget` confirmation, and no Running click, wheel, Enter-toggle or Left/Right expand path.
- `PaneBehavior::DetailTargets` is reachable exactly when `self.panes.targets.content().is_some_and(panes::TargetsData::has_targets)`; `targets_run_visibility` applies the same test. `AppPaneId::Targets` navigates through the shared `navigate_detail` path with Lang, Cpu and Git.
- No process scan exists. `RunningTargetsState`, the `RunningTargetsPolling` startup gate, the cargo workspace index, `ProjectListRevision`, `AcceptedCargoMetadataRevision` and `WorkspaceMetadata.cargo_workspace_root` (with its producer in `scan/cargo_metadata.rs`) are gone. The event loop waits on `app.animation_timeout()` and no longer wakes every 1 s; `FrameMetrics::speed()` returns a `FrameSpeed` (`BelowSlowThreshold` or `Slow`).
- `src/process_observation/` holds only `identity`: `ProcessIdentity`, `observe_current_process_identity` and `revalidate_strong_process_identity`, called from `tui/state/inflight.rs`, `tui/state/owned_run_process_actor.rs` and `tui/terminal/processes.rs`.
- A user keymap entry `kill` gets the existing unknown-entry warning (`ignore_unknown_entries`). CHANGELOG `### Removed` carries a **Breaking** bullet; it and the README send users who want to watch or stop running targets to cargo-tile, installed with `cargo install --git https://github.com/natepiano/cargo-liner cargo-tile`.

**Files:** (under `crates/cargo-port/` unless noted)
- `src/tui/panes/targets/{render,constants,pane,data,mod}.rs` — table-only Targets pane; `TABLE_CHROME` lives only in the `render.rs` test module.
- `src/tui/panes/{mod,actions,system}.rs` — Targets actions without kill or tree toggles; `Panes` without running-target state.
- `src/tui/keymap/{actions,resolved,load}.rs`, `src/tui/integration/framework_keymap/{mod,builder,targets_pane}.rs`, `tests/assets/default-keymap.toml` — keymap without `Kill`.
- `src/tui/app/{mod,construct,confirm_action,tree_mutation}.rs`, `src/tui/app/async_tasks/{dispatch,disk_handlers,metadata_handlers}.rs`, `src/tui/{render,render_context,app_render_state,interaction,background,startup_services,test_support,mod}.rs`, `src/tui/input/dispatch.rs` — app, render, input and startup wiring without running targets, the index or revision counters.
- `src/tui/terminal/{event_loop,frame_metrics}.rs` — animation-timed loop; `FrameMetrics::speed()` → `FrameSpeed`.
- `src/process_observation/{mod,identity}.rs` — process identity for owned runs.
- `src/project/{mod,cargo/mod,cargo/metadata_store}.rs`, `src/scan/cargo_metadata.rs`, `src/tui/project_list/list.rs`, `src/tui/project_list_state.rs` — metadata and project list without the workspace index or revisions.
- `README.md`, `CHANGELOG.md`, `assets/pane-targets-numbered.png` (table plus callouts 1–3); `docs/cargo-port/tooltip.md` without Running references.
- Removed: `src/tui/panes/targets/running_subpane.rs`, `src/tui/running_targets/`, `src/tui/process_refresh.rs`, `src/process_observation/{executor,snapshot}.rs`, `src/tui/workspace_index.rs`, `src/project/cargo/{workspace_index,workspace_index_api_tests}.rs`, `docs/cargo-port/as-built/{targets-running-subpane,running-process-tree}.md`.

**Gotchas:**
- `DOT_CARGO_DIR` in `src/constants.rs` is not scan-only: `lint/trigger.rs`, `watcher/roots.rs` and `project/cargo/metadata_store.rs` read it.
- Pane heights of 2 rows or fewer occur, so the Targets layout carries no minimum-height debug assertion.
- The callouts in `assets/pane-targets-numbered.png` are hand-drawn; no script regenerates them.

**Ruled out:**
- Migrating `kill` entries in user keymaps — the unknown-entry warning covers them.
- Linking cargo-tile on crates.io — the published 0.1.0 lags the workspace version.

