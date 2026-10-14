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

### Phase 2 — cargo-handler paints at once and fills in as data arrives  · status: todo

**Depends on:** Phase 1 (`crates/cargo-handler/src/render.rs`)

#### Work Order

**Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.

**Source:** the user, via the showrunner, 2026-10-14: "cargo handler took about 20 seconds to startup … i think we should stage it so that we show summary and then show cells so we can show information as it comes in". The showrunner's ruling: cargo-handler paints at once. The summary appears first with whatever has been read so far, and each cell fills in as its data arrives. No step waits for all the data before the first frame, and nothing holds the first frame on a slow probe.

**Goal:** the first frame reaches the terminal without waiting on any external process, D-Bus call or file scan. The summary lists agents as soon as the process table is classified, and remotes and per-agent detail arrive after.

**What the 20 s is (read from the code, 2026-10-14; step 1 measures it):**
- `terminal.rs:32` `spawn_appearance_watcher` blocks on `startup_receiver.recv()` (`tui_pane/src/theme/poller.rs:224`). That waits for a D-Bus session and an xdg-desktop-portal `Settings.Read` (`poller.rs:282-316`) with no timeout, before the alternate screen is entered, so nothing is drawn while it waits. This is the prime suspect for a blank 20 s under load.
- Once the attract screen shows (no agents listed yet → `AttractWork::Idle`, `render.rs:54-61`), `before_draw` runs `attract.identify()` (`app.rs:243`) on the UI thread: `kdotool search` (5 s deadline, `tui_pane` `linux/window.rs:465-473`), KWin `getWindowInfo` D-Bus calls with no timeout (`window.rs:494-503`), retried every 500 ms for up to 10 passes. Census answers that have arrived cannot be drawn meanwhile.
- `LocalScanner::scan` (`census/scan.rs:151-213`) sends one `CensusUpdate::Local` at its end, after the `/proc` passes, transcript reads, Codex sqlite, `kwin_evaluate` (5 s deadline, `scan.rs:648`) and `tmux list-panes`/`list-clients` (no timeout, `scan.rs:666`).
- `census/schedule.rs:83-94` runs the local scan before dispatching the ssh probes, so remote time is the local scan plus ssh.

**Spec:**
1. **Measure first.** Add a startup timing log, modelled on cargo-tile's `CARGO_TILE_FRAME_LOG` (`crates/cargo-tile/src/probe.rs:76, 138`), named `CARGO_HANDLER_FRAME_LOG`. It records, in ms since process start: the config load, the appearance read returning, entering the alternate screen, the first frame flushed, each `identify` pass (start and end), the first `CensusUpdate::Local`, and each remote's answer. Run the installed build on natedev with the log on, under the current load. Record the timings and the load average in this plan before changing anything else. The fixes below are kept or dropped by what the log shows.
2. **Appearance never holds the first frame.** Enter the terminal and draw at once with the configured theme (or the last known system appearance). Apply the portal's answer when it arrives, through the watcher that already follows changes. Do not add a timeout in place of this.
3. **Window identification off the UI thread.** `identify` runs on a worker and reports back through a channel that `before_draw` drains. A pass in flight never delays a frame.
4. **Census in stages.** `schedule.rs` dispatches the remote probes before or alongside the local scan. `LocalScanner::scan` sends a first `CensusUpdate::Local` right after `classify::agent_rows` (`scan.rs:165`), with the rows it has, then the full rows when the rest is read. `Census::apply` and `drain_census` already repaint on a change.
5. **Tests:** a test that the first frame is drawn before a slow data source answers. Give the census a source that holds its answer, and assert that the summary frame is in the buffer, with the machine's scanning heading, before the source is released. Add a unit test that the staged scan sends the classified rows before the detailed ones. No test sleeps or takes a second.
6. **Measure again** on natedev under load with the log on. Record time to first frame and time to first agent row, each with the load average beside it.
7. **CHANGELOG** for cargo-handler (and tui_pane, if `poller.rs` or the window identification changes there), under `### Fixed` or `### Changed`.
8. **Retired `/clippy` in two cargo-port doc comments** (showrunner add-on, placed here). natedev retired `/clippy` (~/.claude main 33f5527d), and no outside reader reads cargo-port's `latest.json` marker any more (searched ~/.claude scripts, commands, docs and hooks, and the workspace, 2026-10-14). Reword the doc comments on `RunFinalizeGuard` (`crates/cargo-port/src/lint/runtime/command.rs:200-206`) and `read_status_from_disk` (`crates/cargo-port/src/lint/runtime/supervisor.rs:358-365`) to drop "external readers (the `/clippy` cache check)" and say who does read a stranded `Running` marker: cargo-port's own status read (`read_status_from_disk`) and the lint status it shows. Comment-only; leave the `clippy.log` paths in `history.rs` and `copy_data.rs` alone, as they are log file names.

**Files:**
- `crates/cargo-handler/src/terminal.rs` — no wait on the appearance read before the first frame; the startup log.
- `crates/cargo-handler/src/app.rs` — `before_draw` drains identification results, never runs a pass itself.
- `crates/cargo-handler/src/census/schedule.rs`, `crates/cargo-handler/src/census/scan.rs` — staged answers, remotes not behind the local scan.
- `crates/tui_pane/src/theme/poller.rs` — a non-blocking startup appearance read, only if step 2 needs one there. Claim it through cargo-berth, and send any overlap to the showrunner.
- `crates/tui_pane/src/attract/controller.rs` — identification moved to a worker, only if step 3 needs it there (same claim rule).
- `crates/tui_pane/src/backdrop/desktop/platform/linux/window.rs` — only if step 3 needs it there (same claim rule).
- `crates/cargo-handler/CHANGELOG.md` — a line under `### Fixed`.
- `crates/tui_pane/CHANGELOG.md` — a line, if tui_pane changes.
- `crates/cargo-port/src/lint/runtime/command.rs`, `crates/cargo-port/src/lint/runtime/supervisor.rs` — the two doc comments in step 8.

**Workers:** 1 writer + 1 tester.
- `impl` — every production file above.
- `test` — opens as impl. It writes the first-frame-before-slow-source test and the staged-scan test from this Spec in the modules they cover, messaging `impl` when it needs a hook in production code, such as a census source the test can hold.

**Constraints from prior phases:** Phase 1 added the crate-private `GridLines::open` in tui_pane and the 429×89 gate test `the_live_width_summary_spans_bands_without_an_interior_divider` in `render.rs` (with a `drawn_buffer_in(app, body)` helper). Keep that test green. Phase 1's live capture (installed build, a 429×90 pty, load average 50) saw the first bytes 8 ms after launch and the summary filled within 40 s, so step 1's log decides how much of the 20 s is before the first frame and how much is census.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean, then `verify.sh test cargo-handler` and `verify.sh test tui_pane` green with the new tests. A measured time to first frame on natedev under load, with the load average beside it, recorded before and after in this plan.
