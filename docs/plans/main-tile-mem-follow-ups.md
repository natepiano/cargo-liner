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

#### Work Order

**Goal:** At every width cargo-tile draws a pid whole or not at all, and the two named constants sections are in name order.

**Spec:**

Work only in worktree `/home/natepiano/rust/cargo-liner-tile-mem`, branch `main-tile-mem`.

Today each non-command column asks for `max(header, widest cell)` (`fitted_constraints`, `render.rs` about 1653–1697) and `command` is `Min(7)`. ratatui honours the `Min` before the other columns' `Length`s, so a table narrower than its fitted columns clips them: `start` becomes `sta` and a seven-digit pid becomes its first digits. `table_column_spacing` (1699–1715) picks two cells or one and never checks that the table fits at one. With typical content a command table is whole from an interior width of 61 (69 with `state`), a summary table from 39 (47 with `state`).

Rule: at every width, a pid is drawn whole or not drawn. This holds for the `pid` and `parent` columns, for a row promoted into the summary, and for the ancestry chain over a command table.

- **Tables drop whole columns.** After `visible_columns` (1882–1910) picks the columns, `TableLayout::of` (1412–1439) drops whole columns until the fitted widths plus one-cell gaps fit the table width. Drop order, first to go first: `runs`, `compiler`, `start`, `dur`, `mem`, `cpu`, `state`, `parent`. `pid` and `command` stay while a whole pid, one gap and the `command` minimum fit. Narrower than that, the table shows `pid` alone. Narrower than a whole pid, it shows nothing. The order is a named constant in `crates/cargo-tile/src/constants.rs`, built from the `*_COLUMN` indices. `column_header` and `process_row` already draw only `layout.columns`, so headers are never cut either. `parent` goes last among the droppable columns because a narrow row must keep the identity of the cargo above it.
- **Backstop.** `TableLayout` keeps the solved width of each column (generalize `command_column_width`, 1861–1880, into one solved-width pass). A `pid` or `parent` cell whose solved width is under its text is drawn blank.
- **Ancestry chain.** `ancestry_lines` (1187–1216) draws a level's pid only when the indent plus the whole pid fits the width; otherwise that level's pid is left off (`ancestry_stem`, 1125–1131).
- **Width asks.** `summary_width` (391–442) keeps asking for the full table; dropping columns is what happens when the grid cannot give it.
- **Settings overlay.** Its diagnostics print pids in prose (`settings.rs` 375–415). The overlay breaks a word only when the value column is under seven cells, which a usable terminal never gives. This phase leaves it as it is (author's scope call).
- **Tests** (fast, in-process, in `render.rs`'s `mod tests`, with the existing helpers): two `tight_command_row` fixtures with distinct seven-digit `pid` and `parent`; draw with `draw_process_table` for both `TableKind`s at every interior width from 0 to 90; in the `pid` and `parent` columns' solved rectangles every run of digits equals a whole fixture pid; no header is cut; at 61 and over (command) and 39 and over (summary) every column shows; the drop order is the one above. An ancestry test does the same over widths for the chain.
- **Cost:** at most ten fit checks added per table layout, beside the one spacing check that was already there, and one solve where there was one.

Constants in name order. The rule (`~/rust/nate_style/rust/constants-file-organization.md`): "Within each section, sort constants alphabetically by name." Use plain ascending ASCII order of the identifier. Each doc comment and attribute moves with its constant. No value, type or visibility changes.

- `crates/cargo-tile/src/constants.rs`, `// running-cargo table` (lines 166–429, next section at 431). All `pub(crate) const` but the private `MANIFEST_PATH_FLAG` (309). The Linux constants carry `#[cfg(target_os = "linux")]` (207–219), and `CENSUS_TEST_CADENCE_DIVISOR`, `CENSUS_TEST_CPU_REPORT_MILLIS` and `CENSUS_TEST_CPU_SMOOTHING_MILLIS` each carry `#[cfg(test)]` (279–286): each attribute moves with its constant. `COMPILER_PROCESS_NAMES` uses `RUSTC_BINARY`; a forward reference is legal. `PROCESS_TREE_NOTE_LABEL`'s doc comment (about 408–412) says "Stands alone like the two above it": name them, `ATTRACT_NOTE_LABEL` and `FROZEN_NOTE_LABEL`. Comments that say a column stands beside or ahead of another are about `TABLE_HEADERS` and stay.
- `crates/tui_pane/src/tiles/constants.rs`, `// cell readout` (lines 33–59). Name order: `TILE_FOOT_GAP`, `TILE_FOOT_LEFT_INSET`, `TILE_NUMBER_INDENT`, `TILE_ROWS_CELL_LABEL`, `TILE_ROWS_CELL_SEPARATOR`, `TILE_ROWS_CONTENT_LABEL`, `TILE_ROWS_READOUT_HEIGHT`, `TILE_ROWS_RIGHT_INSET`, `TILE_ROWS_WIDTH_LABEL`.
- The drop-order constant this phase adds goes into the `// running-cargo table` section in name order.

**Files:**
- `crates/cargo-tile/src/render.rs` — whole-column dropping in `TableLayout::of`, the solved-width pass, blank pid cells under their text width, ancestry pids whole or left off, the tests.
- `crates/cargo-tile/src/constants.rs` — the `// running-cargo table` section in name order, the drop-order constant.
- `crates/tui_pane/src/tiles/constants.rs` — the `// cell readout` section in name order.

**Seats:** 2 writers — the work splits by file: the table code with its tests, and the two constants files. The render tests need private items of `render.rs`, so they are written with the code.
- `impl` — `crates/cargo-tile/src/render.rs`
- `test` — opens as impl: `crates/tui_pane/src/tiles/constants.rs`; hub: `crates/cargo-tile/src/constants.rs` (the re-sort, and the drop-order constant `impl` asks for)

**Constraints from prior phases:**
- Phase 1 added `CENSUS_TEST_CPU_REPORT_MILLIS` and `CENSUS_TEST_CPU_SMOOTHING_MILLIS`, both `#[cfg(test)]`, beside `CENSUS_TEST_CADENCE_DIVISOR` in the `// running-cargo table` section; they are sorted with the rest. `READER_TIMESTAMPS_ENV` is in the `// test harness` section and stays where it is.
- Phase 1 changed no drawing code. It added a private `GridMotion` policy in `crates/cargo-tile/src/terminal.rs` whose settle-at-once variant exists only in test builds; this phase does not touch that file.
- The reader scenarios (`crates/cargo-tile/src/shim_registration/reader_scenario.py`, a 240 by 40 terminal) read drawn table columns: `carrier_source_in_rendered` counts the unavailable cells on a row, and the CPU scenario reads the `cpu` column on every completed screen. Dropping columns must leave every column drawn in those cells at that size. `verify.sh test cargo-tile` covers them. Neither seat edits that file; a scenario that fails because a column was dropped is posted to the board for the unit director.
- No test takes a second or more: the width sweeps are in-process and add no process start.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green; `bash ~/.claude/scripts/delegate/verify.sh check tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` and `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` green; the width-sweep tests for both table kinds and the ancestry chain pass; both constants sections are in ascending name order with every value, type and visibility unchanged (`git diff` of each section shows moved lines and the one reworded doc comment only); the real binary beside a real build at 64 columns shows whole pids and whole headers.

### Phase 3 — Reader tests under a second on both systems  · status: todo

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
