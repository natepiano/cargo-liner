# tile-mem follow-ups

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** Every cargo-tile test runs under a second, a pid is drawn whole or not at all, and two constants sections are in name order.

> **As-built disposition: amend** — `docs/cargo-tile/as-built/memory-column.md`, `docs/cargo-tile/as-built/github-runners.md`

> **Production: remove-running-section** — unit `tile-mem-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

Three follow-ups the showrunner gave this unit on 2026-10-07, in this order.

1. Every cargo-tile test that takes a second or more on main: time them, bring each under a second, or report why product behavior sets the floor.
2. "cells narrower still clipped before this phase too; a pid must never be drawn cut so that it reads as a different pid."
3. Re-sort the `// running-cargo table` section of `crates/cargo-tile/src/constants.rs` and the `// cell readout` section of `crates/tui_pane/src/tiles/constants.rs` into name order, with no value or visibility change.

Also from the showrunner, for the as-built amendment: `docs/cargo-tile/as-built/github-runners.md` says `census/mod.rs` "re-exports only thirteen items". It re-exports seventeen; the list lacks `CensusCadence`, `CensusScope`, `ExcludedCommands` and `ProcessOwner`. The amendment also replaces the line in `docs/cargo-tile/as-built/memory-column.md` that says a narrow table truncates headers and cells.

## Delegation Context

- **Project:** `cargo-tile` — a terminal grid of every running cargo command, one cell per command plus a summary cell. Its grid is drawn by `tui_pane`, which `cargo-handler` and `cargo-port` also build on.
- **Project started:** 2026-10-07T23:23:01.306+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-tile-mem`, branch `main-tile-mem`. Every seat works only here. Set by the showrunner (production doc, Units table).
- **Stack:** Rust workspace, ratatui 0.30.2, sysinfo 0.39.6. The reader scenarios are driven by a Python harness run with `uv`.
- **Layout:**
  - `crates/cargo-tile/src/shim_registration/` — tests of the capture shim and of the reader beside live writers.
  - `crates/cargo-tile/src/census/` — the scan worker.
  - `crates/cargo-tile/src/render.rs`, `constants.rs` — the tables and their constants.
  - `crates/cargo-tile/tests/` — integration tests.
  - `crates/tui_pane/src/tiles/constants.rs` — the tile grid's constants.
- **Key files:**
  - `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — starts the Python harness and waits (170–186); reader scope filter (71–75); in-process `settings_click_after_navigation_selects_the_row_still_drawn` (215–272); `reader_child` and `cpu_scan_child` re-exec entry tests.
  - `crates/cargo-tile/src/shim_registration/reader_scenario.py` — fixture tree and shell copy (58–60, 94–98), 24 account directories (81–84), `locale -a` (145–148), `wait_for` (173–184), writer start through the real shim (186–233), carrier (240–293), `end_writer` (295–301), cache server (303–368), `read_terminal` (602–613), `settings_screen` (697–710), resize-and-key proof (729–778) and account walk (781–833), frame self-check exit (929–931), writers per scenario (937–999), reader fork and exec (1007–1017), cleanup (1091–1125).
  - `crates/cargo-tile/src/shim_registration/cargo-capture-shim.sh` — the product shim the scenarios run (read only; no change planned).
  - `crates/cargo-tile/src/shim_registration/wire.rs` — `InstalledShim` setup (31–64), run and utility lookup (89–103, 108–187), mocked `date` (313–340), Darwin observers (388–435), `non_json_arguments_preserve_both_quiet_spellings` (591–596), `darwin_birth_conversion_distinguishes_both_sides_of_dst_fallback` (955–990).
  - `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — `CensusCadence` and its test cadence (183–194).
  - `crates/cargo-tile/src/census/scan.rs` — scan worker sleep (454–469), `scan` (512–551), test `repeated_scans_reuse_roots_resolved_before_an_ancestor_alias_changes` (about 5823–5901), test `spawn_returns_while_root_resolution_waits_and_resolves_once_across_scans`.
  - `crates/cargo-tile/src/progress/capture.rs` — `Capture::take_roots`, account table refresh (154–172).
  - `crates/cargo-tile/tests/shim_modes.rs` — run helpers (204–215, 331–361), `symlinked_capture_directories_preserve_cargo_and_target_contents` (774–805), `no_terminal_returns_cargo_exit_status` (1031–1057), `relative_home_is_omitted_from_registration_but_preserved_for_cargo` (1159–1186), `signal_to_the_shim_alone_waits_for_cargo`.
  - `.config/nextest.toml` — slot rules for the reader tests (1–30). Not changed by this plan.
  - `crates/cargo-tile/src/render.rs` — `summary_width` (391–442), `summary_rows` (648–670), ancestry `Paragraph` (959–982), `ancestry_stem` (1125–1131), `ancestry_lines` (1187–1216), `TableLayout::of` (1412–1439), `fitted_constraints` (1653–1697), `table_column_spacing` (1699–1715), `column_header` (1720–1729), pid and parent cells (1778–1818), `command_column_width` (1861–1880), `visible_columns` (1882–1910), `mod tests` from 2189.
  - `crates/cargo-tile/src/constants.rs` — `// running-cargo table` section (166–429; the next section starts at 431), `TABLE_HEADERS` and the `*_COLUMN` indices, `TABLE_COLUMN_SPACING`, `TIGHT_TABLE_COLUMN_SPACING`.
  - `crates/tui_pane/src/tiles/constants.rs` — `// cell readout` section (33–59; the next section starts at 61).
  - `crates/cargo-tile/src/settings.rs` — pid text in settings diagnostics (375–415). Not changed by this plan.
  - `~/rust/nate_style/rust/constants-file-organization.md` — the constants rule: "Within each section, sort constants alphabetically by name."
- **Test lanes:** `cargo-tile` — `crates/cargo-tile/tests/` (`unit_tests.rs` compiles every `src` module with its `#[cfg(test)]` tests; `shim_modes.rs` and `cli_lifecycle.rs` are integration tests). Render tests live in `render.rs`'s `mod tests` because they need private items. `tui_pane` — `crates/tui_pane/tests/`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - No test takes a second or more. A test that must is named with the product behavior that sets its floor (user rule, memory `no-multi-second-tests`; showrunner follow-up 1).
  - A split or reshaped test still proves what it proved, and its name still says so (author's call).
  - A pid is drawn whole or not at all (showrunner follow-up 2).
  - Every literal the code names lives in the crate's `constants.rs`, in name order within its section (style guide, `constants-file-organization.md`).
  - `tui_pane`'s public API does not change (author's call, so `cargo-handler` and `cargo-port` compile untouched).
  - Format with `cargo +nightly fmt` only (user rule, memory `rustfmt-nightly-only`).
  - No UX guide covers cargo-tile (production doc, Production rules), so no UX check lines.
  - Saved run output stays small: read each timing run and delete it before the next (user rule, 2026-10-04).

## Phases

### Phase 1 — Fast cargo-tile tests  · status: done

#### As-built

- The shim signal tests wait on a handshake with the fixture cargo: it writes `cargo-held`, waits for `release-cargo`, then writes `cargo-finished`. `run_detached_after_start` runs the check between the two. Each of the three tests takes about 0.06s.
- `darwin_birth_conversion_distinguishes_both_sides_of_dst_fallback` asserts in process that the two epochs are an hour apart and give equal local time.
- `spawn_resolved_scan_loop` (private, `census/scan.rs`) is the scan worker's loop. `spawn_with_resolver` calls it, and the worker test drives it with a cheap scanner and asserts one root resolution across two scans.
- `CensusCadence::for_test` polls and reports every 50 ms and smooths over 100 ms, from `CENSUS_TEST_CPU_REPORT_MILLIS` and `CENSUS_TEST_CPU_SMOOTHING_MILLIS`.
- The reader scenarios use a shell carrier with a seeded registration and read readiness from the drawn row at each source. `CompletedTerminalFrames` queues every completed screen; the CPU scenario checks each one in its 250 ms window and starts no extra process.
- `GridMotion` (private, `terminal.rs`) is `Animated` in the shipped binary. A `#[cfg(test)]` `Immediate` variant settles tile motion at once, for the source-switch reader only.

**Files:**
- `crates/cargo-tile/tests/shim_modes.rs` — the shim signal handshake tests
- `crates/cargo-tile/src/shim_registration/wire.rs` — the in-process Darwin birth-time test
- `crates/cargo-tile/src/census/scan.rs` — the scan loop and its worker test
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — the test cadence
- `crates/cargo-tile/src/constants.rs` — the two test cadence constants and `READER_TIMESTAMPS_ENV`
- `crates/cargo-tile/src/shim_registration/reader_scenario.py`, `reader_scenarios.rs` — the reader scenarios and their entry tests
- `crates/cargo-tile/src/terminal.rs` — the motion policy

**Binds later work:** the reader scenarios read drawn table columns in a 240 by 40 terminal, so a change to which columns a table draws must leave them whole at that size. The two test cadence constants sit in the `// running-cargo table` section of `constants.rs`.

**Gotchas:**
- A reader scenario that waits for a row inside a newly opened tile pays the grid's 720 ms opening animation unless its reader uses the settle-at-once policy.
- `reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` has a floor of the 250 ms window plus the reader's 0.3s start: 0.67s alone, and over a second once under heavy machine load.
- The shell handshake, the ended-pid choice and the in-process Darwin test were first run on Linux only.

**Ruled out:**
- A separate scan-watching child process for the CPU scenario: it cost a process start, and checking every completed screen proves the same.
- Treating the source-switch scenario's first second as a product floor: it was presentation time.

### Phase 2 — Whole pids and ordered constants  · status: done

#### As-built

- `TableLayout::of` drops whole columns in `TABLE_COLUMN_DROP_ORDER` (runs, compiler, start, dur, mem, cpu, state, parent) until the fitted widths plus one-cell gaps fit; then `command`, then `pid`. Headers and cells are never cut.
- `TableLayout` keeps one solved width per kept column (`column_widths`); `TableLayout::column_width` is 0 for an omitted column. A `pid` or `parent` cell narrower than its text is drawn blank (`number_text_if_fits`).
- A table that has rows and keeps no column draws `TABLE_NO_COLUMNS_MARKER` (`…`) at the start of its first row, in the header style (`column_header_style`), and nothing else; `table_height` asks for that one row. A table with no rows, or with no interior area, draws no marker.
- `ancestry_stem` leaves a level's pid off when the indent plus the whole pid does not fit.
- The `// running-cargo table` section of cargo-tile's constants and the `// cell readout` section of tui_pane's tile constants are in ascending name order; `PROCESS_TREE_NOTE_LABEL`'s doc names `ATTRACT_NOTE_LABEL` and `FROZEN_NOTE_LABEL`.

**Files:**
- `crates/cargo-tile/src/render.rs` — column dropping, solved widths, the no-room marker, blank undersized pid cells, ancestry pids, the width-sweep and marker tests
- `crates/cargo-tile/src/constants.rs` — `TABLE_COLUMN_DROP_ORDER`, `TABLE_NO_COLUMNS_MARKER`, the section in name order
- `crates/tui_pane/src/tiles/constants.rs` — the section in name order

**Gotchas:**
- A narrow layout runs up to eleven fit scans: ten from column dropping beside the spacing check that was already there.
- A width sweep of 0 to 90 in one test passes a second inside the full suite; the sweeps are split into four width ranges per table kind.
- With typical content a command table shows every column from an interior width of 61 (69 with `state`), a summary table from 39 (47 with `state`). The real binary shows `pid command` at 64 terminal columns, `pid` alone at 48, and the marker in tiles whose interior is under 8 cells (seen at 24 columns).
- The grid's column count varies with how many commands run, so one terminal width does not always reach the no-room state: a capture that must show the marker needs a tile interior narrower than the indent plus a whole pid.
- `ANCESTRY_ELISION` and `TABLE_NO_COLUMNS_MARKER` are the same character; the ancestry one sits at the chain's indent, the table one at the start of the header row.

**Ruled out:**
- Treating the eleventh fit scan as a defect: it is the earlier spacing check, not added work.
- Changing the settings overlay's pid prose: it breaks a word only under seven value cells, which a usable terminal never gives.
- Drawing nothing for a table with rows and no room: an occupied tile then reads as empty.

### Phase 3 — Reader tests under a second on both systems  · status: done

#### As-built

- Every cargo-tile reader test passes in under a second run alone on Linux and macOS. The slowest is the compiler-cache CPU test (0.70s on Linux, 0.93s on macOS), whose 0.26s CPU observation window is the accepted floor.
- `link_named_shell` signs one copy of the shell per scenario (`cargo-tile-real`) and hard-links every other name to it.
- The CPU scenario starts its cache server and both writers together. `start_reader_process` starts the reader only after both invocation CPU baselines exist.
- The source-switch scenario wakes its fixture shells through a named pipe: `WriterNotification` (`send`, `close`) and `trigger_writer(observations, trigger)`. A shell reads `$OBSERVED/notification` when the pipe exists and sleeps 20 ms otherwise. Its reader starts before the registration carrier.
- A scenario deadline (`signal.setitimer`) raises `TimeoutError` naming the wait still pending in any thread (`pending_waits`), and a hard stop follows it. `reader_scenario_deadline_names_a_wait_still_pending_in_another_thread` pins it.
- `CARGO_TILE_READER_TIMESTAMPS=1` prints a timestamp per fixture step, cleanup steps included.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario.py` — the reader scenarios and their fixture.
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — the runner that starts the script and passes its deadline.

**Gotchas:**
- On macOS a reader started before both CPU baselines loses the CPU row's reading; Linux does not show it.
- A signed copy of the shell costs about 0.025s on macOS. Fixed 20 ms shell polls and serial starts were the cost of fixture preparation.
- A fixture shell that reads the named pipe needs the sleep fallback: with the pipe gone, a bare read loop spins a core.
- Linux timings do not predict macOS. A cut is measured on the Mac in turn with the version before it.
- Inside the macOS suite under load four reader tests take 1.2 to 1.8s (excluded command, foreign owner, quiet JSON, CPU); alone none does. A source-switch run alone on a loaded Mac is occasionally just over a second, in the reader's own steps.
- A ten-second source-switch failure showed twice on macOS and not again in about 300 runs; its cause is unproven.

**Ruled out:** a shell warmup before the scenario (no gain on macOS); codesign as the cost of fixture preparation.

### Phase 4 — Mem total whole or absent  · status: done

#### As-built

- The summary tile's foot text (cargo-tile's memory total, `mem 12.3G`) is drawn whole, value with its unit, or not at all.
- `draw_summary_foot` returns the private `SummaryFootDrawOutcome`: `NotDrawn` when the text is wider than `inner.width - TILE_FOOT_LEFT_INSET` or the tile has no readout row, `DrawnWhole { width }` otherwise.
- `draw_cell` maps `SummaryFoot::Empty` to `NotDrawn`. `NotDrawn` takes `draw_rows_readout`, so a summary tile with no room for its foot draws exactly the row a tile with no foot draws; `draw_rows_readout_after_foot` runs only for `DrawnWhole`.
- The doc comments on `SummaryFoot::Text` and `TileCells::summary_foot` and the unreleased `tui_pane` changelog entry state the rule. No public signature changed; cargo-tile's `SummaryMemoryTotal::foot` is unchanged.
- `summary_foot_is_drawn_whole_or_absent_at_every_width` sweeps every summary inner width from 0 to the first width that shows foot and readout side by side: a non-fitting width equals a `SummaryFoot::Empty` render cell for cell, a fitting width holds the exact foot text.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — the draw step, `SummaryFootDrawOutcome`, and the width sweep in its test module
- `crates/tui_pane/CHANGELOG.md` — the unreleased `SummaryFoot` entry

**Gotchas:**
- The rows readout is still cut by a cell narrower than its text (`draw_rows_readout_line` clips). With the foot absent, a narrow summary tile shows that cut readout: `content` at 30 terminal columns, `conten` at 24.
- The fit test uses `Line::width()`. No app writes non-ASCII foot text; a text that measure reports narrower than the renderer draws could still be cut.

**Ruled out:** a shorter form of the total for narrow tiles; guarding the fit test against text whose measured width differs from its drawn width.

