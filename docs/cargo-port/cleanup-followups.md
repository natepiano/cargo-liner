# cargo-port cleanup follow-ups

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Fixes the defects the Running-section removal surfaced, which predate it, before cargo-port 0.8.0 ships.

> **Production: cargo-port-cleanup** — unit `cleanup-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Delegation Context

- **Project:** cargo-port — terminal UI for browsing and running Rust projects
- **Project started:** 2026-10-06T21:06:33.289+00:00
- **Stack:** Rust 2024, ratatui, crossterm, the workspace's `tui_pane` framework
- **Layout:** `crates/cargo-port/src/tui/input/` (key dispatch), `crates/cargo-port/src/tui/app/mod.rs` (App and its in-file test module), `crates/cargo-port/src/tui/panes/targets/` (Targets pane), `crates/cargo-port/README.md`, `crates/cargo-port/CHANGELOG.md`
- **Key files:** `crates/cargo-port/src/tui/input/dispatch.rs` — app-surface key dispatch; `classify_output_cancel_preflight` decides what `Esc` does before any overlay sees it
- **Key files:** `crates/cargo-port/src/tui/app/mod.rs` — in-file tests that press keys through production input (`press`, `press_key`, `make_app_with_keymap_toml`)
- **Key files:** `crates/cargo-port/src/tui/panes/targets/data.rs` — `TargetsData::from_workspace_metadata`, `TargetSource`, `TargetSourceKind`
- **Test lanes:** cargo-port — none for this work (the dispatch tests live in the in-file test module of `src/tui/app/mod.rs`)
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-port`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-port`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:** an owned run is stopped only by a key the user aimed at the run; any open overlay owns its keys first

## Phases

### Phase 1 — Esc respects open overlays; README and comment corrections  · status: done

#### As-built

- `classify_output_cancel_preflight` in `src/tui/input/dispatch.rs` reads all three preflight outcomes from one local, `can_handle_output_cancel = !app_overlay_open && !focused_text_input_mode(app)`, and `running_example` requires it. `Esc` with the finder or sccache overlay open, or a focused text input active, falls through to that overlay's dispatch and the owned run keeps going; with nothing open, `Esc` stops the run through `terminal::signal_owned_run`.
- `esc_with_finder_open_closes_finder_and_keeps_run_going` and `esc_with_sccache_open_closes_sccache_and_keeps_run_going` press `Esc` through production input and assert the overlay closes, `example_running()` stays `Some`, and `owned_run_termination()` is not `RequestPending`.
- README Targets list names binaries, examples and benches; Targets item 3 says a run's output streams into the Output pane along the bottom and `Esc` stops the run, with a blank line before the cargo-tile paragraph.
- The `from_workspace_metadata` doc comment: `TargetSource::workspace_root` only for a multi-package workspace's root-manifest package, `TargetSource::member` for every other package, standalone included; the Source column shows the package name either way and the kind only orders rows.

**Files:**
- `crates/cargo-port/src/tui/input/dispatch.rs` — `Esc` run-stop gated on no overlay and no focused text input
- `crates/cargo-port/src/tui/app/mod.rs` — the two regression tests; `lint_cell` doc names `tui/panes/project_list/`
- `crates/cargo-port/src/tui/integration/framework_keymap/mod.rs` — module doc names `src/tui/keymap/`
- `crates/cargo-port/src/tui/panes/targets/data.rs` — `from_workspace_metadata` doc comment
- `crates/cargo-port/CHANGELOG.md` — `### Fixed` bullet for the `Esc` overlay fix
- `crates/cargo-port/README.md` — Targets list, Targets item 3, list break
- `docs/cargo-port/as-built/lint-file-lock-spinner.md` — as-built doc for the lint file-lock spinner; replaces `blocked-lint-spinner.md`
- `docs/cargo-port/style/adding-a-keybinding.md` — checklist points at the framework keymap files

**Gotchas:**
- The output-cancel preflight runs before `dispatch_finder_overlay` and `sccache::dispatch_sccache_overlay` in `handle_app_surface_key`; any new preflight outcome must read `can_handle_output_cancel` or it steals keys from open overlays.
- The sccache regression test opens the overlay with `app.overlays.open_sccache()`, not `open_sccache_stats_overlay`, so no background sccache request starts.

