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

#### Work Order

**Goal:** Every cargo-tile reader test passes in under a second alone, on macOS and on Linux.

**Spec:**

Work only in worktree `/home/natepiano/rust/cargo-liner-tile-mem`, branch `main-tile-mem`. Test code only: no product file changes.

The showrunner's scope: "measure first, then cut fixture preparation where the measurement points; target every reader test under a second alone on both macOS and Linux, with the 0.26s CPU observation as the only accepted floor; report in-suite times on both systems after it."

Measured 2026-10-07, on the tree at `f2bdaaed`:

- Alone on a quiet Mac: `reader_keeps_parent_family_across_its_only_childs_source_switch` 0.98 to 1.18s; `reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` 1.30s. Alone on Linux the CPU test took 1.22s and the source switch 0.44 to 0.83s.
- Source switch on the Mac, from the harness timestamps (`CARGO_TILE_READER_TIMESTAMPS=1`): first writer up at 0.20s (0.045s on Linux), reader start to first frame 0.14s, the two switches 0.29s, cleanup 0.13s; the script ends at 0.87s and the test at about 1.0s.
- CPU test on the Mac: cache server up at 0.15s, first writer at 0.31s, second writer at 0.60s, reader start 0.67s, first frame 0.82s, CPU rows 0.87s, CPU observation ends 1.13s (0.26s, the accepted floor), cleanup 0.10s.
- On the Mac, `codesign` of a copied `/bin/sh` takes 24 ms. The cost is the first run of each newly signed copy: copy, sign and first run take 119 to 266 ms, against 5 ms for a later run of the same file. `copy_named_shell` makes such a copy three times in the script. `python3 -c pass` takes 18 ms and `ps -p … -o lstart=` 4 ms.
- Inside the full suite several reader tests pass over a second. Mac: CPU 1.8 to 2.0s, excluded command 1.4 to 1.6s, foreign owner 1.2 to 1.3s, source switch 0.9 to 1.1s. Linux: CPU 1.8s, source switch 1.2s, locale 1.0s.

Work:

- **Measure before each cut.** Add a timestamp at each fixture step that has none (each `copy_named_shell`, each writer's first output, the cache server's readiness), run the two tests, and cut only what the timestamps show. Put the before and after numbers for each cut in the summary.
- **First run of a signed copy.** Find a way for the named shells to skip the first-run cost: one signed copy per scenario reused through hard links or `exec -a`-style naming, or the copies made and run once in parallel with the rest of the preparation. Whatever is chosen, the process must still appear under its cargo or compiler name to the reader, which is what the copy exists for. Prove each option on the Mac before keeping it.
- **Preparation in parallel.** The CPU test starts its cache server and two writers one after another (0.60s on the Mac). Start what does not depend on each other together.
- **The floor.** The CPU observation window (`cpu_observation_window` of the test cadence) stays as it is. No assertion is weakened and no wait is replaced by a fixed sleep.
- **The hang repair stays.** `end_reader`, the bounded waits, the scenario deadline and its hard stop, and the two tests that pin them are not loosened. Cleanup may get faster only by doing less waiting on work that has already ended.
- **In-suite times.** After the cuts, run the whole cargo-tile suite on both systems and report each reader test's time. If a test is still over a second only inside the suite, say what it waits on there; do not edit `.config/nextest.toml`.

The Mac: the unit director runs the Mac measurements and sends the numbers to the seat. The seat writes on Linux and never assumes a Mac result.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario.py` — fixture preparation, timestamps.
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — only if the runner's own start-up shows in the measurement.

**Seats:** 1 writer — the scenario script cannot split, and the tests are the script itself.
- `impl` — `crates/cargo-tile/src/shim_registration/reader_scenario.py`; hub: `crates/cargo-tile/src/shim_registration/reader_scenarios.rs`
- `test` — opens as impl: no files; this phase leaves the seat idle

**Constraints from prior phases:**
- The fix commit `f2bdaaed` made every reader scenario end itself: a scenario fails at 20s naming what it waited for, stops hard at 25s, and nextest ends a reader test at 40s. On macOS a process cannot finish exiting while its terminal holds unread output, so the script must keep reading the reader's terminal until the reader has exited.
- Phase 1 gave the source-switch reader a test-only motion policy that settles tile motion at once; without it the tile-opening animation costs 0.72s.
- Phase 2 drops whole table columns when a table is too narrow. The scenarios run at 240 by 40, where every column is drawn.
- These tests also run on macOS in CI: no GNU-only tool use.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green; every `shim_registration::reader_scenarios::` test under a second alone on Linux and, run by the unit director, on the Mac; the once-hung test passes 150 times under CPU load on the Mac; in-suite times for both systems are in the checkpoint notice.

### Phase 4 — Mem total whole or absent  · status: todo

#### Work Order

**Goal:** The summary tile's memory total is drawn whole, value with its unit, or not at all, at every width.

**Spec:** Work only in worktree `/home/natepiano/rust/cargo-liner-tile-mem`, branch `main-tile-mem`.

The showrunner's routing, verbatim: "item 4 (the summary's mem total losing its unit at 30 and 24 columns) is yours as phase 4 ... Phase 4: the mem total is drawn whole, value with its unit, or not at all, at every width; a capture and a design check pass as for phase 2. The "…" marker stands."

What is wrong today. `draw_summary_foot` (`crates/tui_pane/src/tiles/draw.rs`) draws the app's foot text at the left end of the summary tile's readout row and clips it to the row: `width = line.width().min(inner.width - TILE_FOOT_LEFT_INSET)`. In the real binary at 30 terminal columns the total reads `mem 24.6` and at 24 columns `mem 23`: a value with its unit cut off, which reads as a different measurement.

Intended behavior:
- When the foot line's whole width fits in `inner.width - TILE_FOOT_LEFT_INSET`, it is drawn as today.
- When it does not fit, `draw_summary_foot` draws nothing and returns 0, so `draw_rows_readout_after_foot` lays the rows readout out exactly as it does with no foot.
- The rule lives in `draw_summary_foot`, the one place the foot is drawn. No caller measures the text, and cargo-tile's `SummaryMemoryTotal::foot` (`crates/cargo-tile/src/render.rs`) is unchanged.
- No shorter form is drawn: no value without its unit, no label alone.
- `SummaryFoot`, `TileCells::summary_foot` and every public signature of `tui_pane` stay as they are.

Tests, in `draw.rs`'s `mod tests`, fast and in-process, with the existing helpers (`FOOT_TEXT`, the test `TileCells`): at every summary inner width from 0 up to the width that shows the foot and the readout side by side, the readout row holds the whole foot text or none of its characters; at a width one cell short of the foot the readout is drawn as it is with `SummaryFoot::Empty`. Update `summary_foot_wins_when_readout_does_not_fit` only if its width no longer holds the whole foot.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — `draw_summary_foot` and its tests.

**Seats:** 1 writer — one function and its tests in one file.
- `impl` — `crates/tui_pane/src/tiles/draw.rs`.
- `test` — opens as impl: no files; this phase leaves the seat idle.

**Constraints from prior phases:**
- Phase 2: a table too narrow for a whole pid draws one `…` (`TABLE_NO_COLUMNS_MARKER`); that marker stands and this phase does not touch it or `crates/cargo-tile/src/render.rs`.
- Phase 2: the grid's column count varies with how many commands run, so one terminal width does not always reach a given tile width; the capture that proves this phase needs a summary tile interior narrower than the total's text (seen at 30 and 24 terminal columns).
- `tui_pane` is shared with cargo-handler and cargo-port: no public signature changes, and both crates' tests run before the checkpoint.
- No test may take a second or more.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` and `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` green; `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` green, run by the unit director; captures of the real binary at 30 and 24 terminal columns show the total whole or absent; a design check by a fresh helper passes on those captures.
