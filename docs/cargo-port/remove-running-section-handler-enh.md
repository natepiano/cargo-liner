# handler-enh

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** A summary that spans several tile columns reads as one unbroken pane: no column divider is drawn through it.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-14, the user, relayed by the showrunner: "while it figured out that it needed to show the summary across two tiles - it still draws the column divider line - obscuring actual text in the summary rows that i don't want obscured - have the handler enh fix this first".

The showrunner's ruling: when one pane spans tiles, nothing is drawn between them. The summary rows read as one unbroken line, and each column keeps its usual spacing. Gate: a render test at the live width, with a spanning summary, that asserts no divider cell inside a summary row, and a shot of the live handler.

Shot: the live handler at 429×90 with `tiles.widen_summary = true`. The summary spans two columns, and the line between them overwrites the directory column, so "~/rust │ hana_catalyst" and "direct│ry" appear.

## Cause

`tui_pane::tiles::draw::draw_placements` (`crates/tui_pane/src/tiles/draw.rs:325`) adds every column band's whole frame to `GridLines` (`grid_lines.add(PaneFrame::new(column))` for each `drawing.column_bands`). Each band runs the full height of the grid. When the summary is widened across bands (`summary_span > 1`, `grid.rs:3440`), the vertical edge two bands share runs straight through the summary's interior. The summary's contents are drawn first and the lattice goes over them, so that line covers one character of every summary row. The fault is in `tui_pane`, not in cargo-handler: cargo-handler draws the summary at the full spanned width.

## Delegation Context

- **Project:** `tui_pane`, the shared terminal-pane framework whose tile grid draws cargo-handler's cells, and `cargo-handler`, the agent watcher that draws its summary in that grid.
- **Project started:** 2026-10-14T19:43:48.762+00:00
- **Stack:** Rust, ratatui, cargo-nextest.
- **Test lanes:** `crates/tui_pane/tests` exists but the grid's tests are module tests; `cargo-handler` has none. This phase's tests are module tests.
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Key files:**
  - `crates/tui_pane/src/pane/frame.rs`: `GridLines` (400), with `add` (437), `add_rule` (508), `render` (559), `mark` (692), `cell_at` (703), `run` (726) and `glyph` (738). `Sides`/`Side` hold up, right, down and left bits.
  - `crates/tui_pane/src/tiles/draw.rs`: `draw_placements` (325). The `TileContent::Summary` arm adds the summary's `line_frame`. The test helper `is_lattice_glyph` and the test `a_grid_too_wide_for_its_window_draws_the_summary_alone` (≈4843) already assert that no divider appears inside a summary-only frame.
  - `crates/cargo-handler/src/render.rs`: the tests module (≈230) with `laid_out_app`, `boss`, `drawn_buffer` (80×24 body) and `Cells`.
- **Overlap:** `crates/tui_pane` is held by tile-fixes (reservation `01a13687-deb4-7be3-aa77-c595d43377af`). The showrunner answered `--before`: this unit lands first, and tile-fixes, which has uncommitted edits in `frame.rs` and `draw.rs`, resolves any conflict. Edits stay on the named lines.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check tui_pane`, and `… check cargo-handler`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `… test cargo-handler`; the production's merge tests also run `cargo-tile` and `cargo-port`, which build on tui_pane.
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` (one run covers the workspace)
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:** tests run with nextest. Format with `cargo +nightly fmt` only. No test takes a second. Shared borders stay one line wide, and their crossings still resolve through `glyph`.

## Phases

### Phase 1 — A spanning summary has no divider through it  · status: done

**Depends on:** none

#### As-built

- `GridLines::open(&mut self, rect: Rect)` (crate-private, `frame.rs`, beside `add_rule`) opens one pane that spans its neighbours' bands: every cell strictly inside `rect` loses all sides, each edge cell loses its inward side, and corners keep their joins. `Sides::without(side)` supports it.
- `draw_placements` collects each visible summary piece's `line_frame.rect()` and opens them after every band and piece line is added, before `render`, so a widened summary (`summary_span > 1`) reads as one unbroken pane and its bottom border keeps `┬` where a column line continues below.
- Tests: `a_spanning_pane_opens_band_lines_and_keeps_border_joins` (frame.rs), `a_widened_summary_has_no_interior_band_divider` (draw.rs), and `the_live_width_summary_spans_bands_without_an_interior_divider` (cargo-handler `render.rs`, 429×89, reading the summary's rect off the drawn `┌ summary` corner; helper `drawn_buffer_in(app, body)`).

**Files:**
- `crates/tui_pane/src/pane/frame.rs` — `Sides::without`, `GridLines::open`, its test.
- `crates/tui_pane/src/tiles/draw.rs` — opens summary frames before render; the widened-summary test.
- `crates/cargo-handler/src/render.rs` — the 429×89 test and `drawn_buffer_in`.

**Gotchas:** `open` must run after every `add`; a band added later puts the line back.

### Phase 2 — cargo-handler paints at once and fills in as data arrives  · status: done

**Depends on:** Phase 1 (`crates/cargo-handler/src/render.rs`)

#### As-built

- The census scheduler (`spawn_staged_with`, `census/schedule.rs`) dispatches the remote ssh probes before the local scan in each round, so remotes never wait behind it. Its local source takes the `Sender<CensusUpdate>` and may publish more than once per scan; `spawn_with_test_source` lets a test hold that source.
- `LocalScanner::scan_staged(&mut self, publish: impl FnMut(Vec<AgentRow>))` reads sessions, the process table and Codex sqlite, then calls the method `scan_stages`, which publishes the classified rows from `classify::agent_rows`, reads the details (`attach_details`) and publishes the completed rows. `LocalPublication` (`FirstAnswerPending` → `CompletedAnswerPublished`) limits the classified-row publication to the first scan; each later scan publishes completed rows only, so a refresh never shows agents without branches, launchers and children. `scan` keeps only the completed answer for the wire probe.
- `AgentRow.process_started` and `AgentCell.process_started` hold the process's own start time and key the cell, apart from `started`, which `attach_session_starts` moves back to the transcript's first line for the age shown; a resumed session keeps its cell and focus. `ProbeAgentRow` (`census/probe.rs`) fills a missing `process_started` from `started` for older probes, still schema 3.
- `startup_log.rs` writes ms-since-start milestones (config, appearance read, first frame, each `identify` pass while the attract screen shows, first local answer, first agent rows, each applied update) and per-stage `scan_stage_start`/`scan_stage_end` lines via `startup_stage`, to the path in the env var named by `FRAME_LOG_VARIABLE` (`CARGO_HANDLER_FRAME_LOG`). `StartupProbe` is the app's and attract screen's `FrameProbe`.
- Tests: `the_first_frame_draws_the_scanning_summary_before_a_slow_source_answers` (`terminal/startup_tests.rs`), `the_staged_scan_sends_classified_rows_before_reading_details` and `later_scans_publish_completed_rows_without_a_classified_refresh` (`census/scan/staged_tests.rs`), `a_resumed_claude_session_keeps_its_cell_key_when_its_age_moves_earlier` and `older_probe_rows_keep_pid_reuse_protection_from_their_start_times` (`agent_cell.rs`).
- The doc comments on `RunFinalizeGuard` and `read_status_from_disk` in cargo-port name cargo-port's own status read as the reader of a stranded `Running` marker.

**Files:**
- `crates/cargo-handler/src/startup_log.rs` — the startup timing log and `StartupProbe`.
- `crates/cargo-handler/src/census/schedule.rs`, `census/scan.rs` — remotes first, staged local answers, `LocalPublication`.
- `crates/cargo-handler/src/census/mod.rs`, `census/classify.rs`, `census/probe.rs`, `agent_cell.rs`, `tiles.rs` — `process_started` cell identity and its probe fallback.
- `crates/cargo-handler/src/terminal.rs`, `app.rs` — log milestones; `before_draw` identifies only while the attract screen shows.

**Gotchas:** The slow first scan is two cold first reads in the details stage: `transcript_launchers` (4.4 s cold, 5 ms warm) and `codex_app_server_threads` (2.4 s cold, 3 ms warm); under load 162 they made a 36 s first answer. With staging, the first frame lands at ~11 ms and the first agent rows at ~118–228 ms under load, with details seconds later. Keying a cell by `started` flips a resumed session's key when its transcript start arrives.

**Ruled out:** Making the appearance read non-blocking (it costs 30 ms); moving window identification off the UI thread (each pass ≤119 ms).
