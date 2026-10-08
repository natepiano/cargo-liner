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
  - `crates/cargo-tile/src/constants.rs` — `// running-cargo table` section (161–420; the next section starts at 422), `TABLE_HEADERS` and the `*_COLUMN` indices, `TABLE_COLUMN_SPACING`, `TIGHT_TABLE_COLUMN_SPACING`.
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

#### Work Order

**Goal:** No cargo-tile test takes a second or more in `verify.sh test cargo-tile` on the loaded machine, and each test named below runs in about half a second or less.

**Spec:**

Work only in worktree `/home/natepiano/rust/cargo-liner-tile-mem`, branch `main-tile-mem`.

Measured 2026-10-07 16:15 PDT with `verify.sh test cargo-tile`, machine load average about 41: 749 tests passed. Five took a second or more, all in `shim_registration::reader_scenarios`: `reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` 2.9s, `reader_keeps_one_direct_row_for_quiet_json` 1.5s, `reader_keeps_parent_family_across_its_only_childs_source_switch` 1.4s, `reader_ignores_foreign_owned_account_and_reports_its_owner_in_settings` 1.4s, `reader_excludes_a_live_command_without_sweeping_its_capture` 1.3s. Under the heavier load of a merge run these also crossed a second: `wire::darwin_birth_conversion_distinguishes_both_sides_of_dst_fallback` 2.1s, `census::scan::tests::repeated_scans_reuse_roots_resolved_before_an_ancestor_alias_changes` 2.0s, three `shim_modes` tests 1.7–1.9s, `wire::non_json_arguments_preserve_both_quiet_spellings` 1.7s. At load 41 those ran at 0.4–0.75s, as did `reader_opens_settings_when_resize_and_key_are_pending_together` 0.73s, `reader_accepts_live_writer_under_different_locale_and_timezone` 0.55s, `spawn_returns_while_root_resolution_waits_and_resolves_once_across_scans` 0.64s and `signal_to_the_shim_alone_waits_for_cargo` 0.75s.

Target: no cargo-tile test at or over one second, and each test named here at or under about half a second, so heavier load does not push it back over. A test left over a second is reported by name with the product behavior that sets its floor. No test in this list sleeps on purpose: the time is process starts and repeated setup. No product behavior changes; the only product-side code is the `#[cfg(test)]` cadence.

Reader scenarios (`reader_scenarios.rs`, `reader_scenario.py`). Each scenario starts Python (`reader_scenarios.rs` 170–186), builds a fixture tree and copies a shell as `cargo-tile-real` (`reader_scenario.py` 58–60, 94–98), runs `locale -a` (145–148) whether or not locale is under test, starts each writer through the real shim (186–233; a shell, the shim, its setup utilities, a fake cargo, `tee`), and forks a PTY that re-execs the test binary as `reader_child` (1010–1017). Each scan refreshes every process on the host (`census/scan.rs` 512–516). Test cadence is production divided by five: 50 ms poll, 200 ms cpu report, 400 ms smoothing (`invocation_cpu_accounting.rs` 183–194).

1. First add phase timestamps to the Python harness (Python start, each writer, reader exec, first frame, assertions, cleanup), printed only on request, so each change below is measured. They stay in the harness.
2. Run `locale -a` only for the locale scenario. Move the terminal-frame self-check ahead of fixture and locale setup (`reader_snapshots_publish_only_completed_terminal_frames` exits at 929–931 after paying for both).
3. Locale scenario: every writer already runs under the differing locale and timezone, so fold its assertions into another live-reader scenario and drop its own reader launch. The test `reader_accepts_live_writer_under_different_locale_and_timezone` keeps its name and still fails when a writer under a different locale or timezone is not accepted.
4. Foreign-owner scenario: copy the first live writer's publication into the foreign root with the foreign metadata in place of starting a second writer (983–984). It still proves a foreign-owned publication cannot relabel a live cargo pid.
5. Resize-and-key scenario: end the PTY test once the queued resize and key open settings (729–778). Move the walk over 24 accounts and five settings (781–833) to an in-process `TestBackend` test beside `settings_click_after_navigation_selects_the_row_still_drawn` (`reader_scenarios.rs` 215–272).
6. Cpu-attribution scenario: the only one with a real floor, smoothing plus three report windows. Give it a `#[cfg(test)]` cadence through `CensusCadence`: 50 ms poll, 50 ms report, 100 ms smoothing, so the window is 250 ms and a sample still spans five Linux clock ticks. Any number it needs is a named constant in `crates/cargo-tile/src/constants.rs`, beside `CENSUS_TEST_CADENCE_DIVISOR`.
7. Quiet-JSON scenario: make the quiet writer the first writer in place of a generic writer followed by a quiet one (937, 971–979).
8. Excluded scenario: make the excluded enclosing writer the first writer and seed the ended sibling's registration and log directly with a known-ended pid (985–999, 295–301). Both live nested commands stay.
9. Source-switch scenario (`reader_keeps_parent_family_across_its_only_childs_source_switch`, 240–293, 671–684): use the timestamps to find what it waits on and cut it; the shared changes above apply.
10. `.config/nextest.toml` stays as it is. A test's reported time starts when the test starts, so the slot rules add queueing, not measured time, and the all-slots rule was added for a real flake.

`wire::non_json_arguments_preserve_both_quiet_spellings` (`wire.rs` 591–596): three `assert_executed_arguments` calls, each with a new temporary installation (31–64), a utility-locator shell (96–103) and a full shim run. Share one installation and one utility lookup across a test's runs, each run in its own observations subdirectory, so each spelling is still proved alone. Use the same helper in the other multi-run `wire` tests (`json_arguments_preserve_both_quiet_spellings_after_separator`, `darwin_birth_conversion_scopes_locale_and_keeps_cargo_environment`, and any other test that installs more than once).

`wire::darwin_birth_conversion_distinguishes_both_sides_of_dst_fallback` (955–990): two full setups and six Python launches. One installation; the two real shim runs stay; the standalone Python wall-time calls (962–969) go, replaced by an in-process conversion or by the mocked `ps` adapter recording the local time it produced; a shell observer serves the two fixed timestamps where Python is not needed. The test must still show the two epochs share local 01:30 and publish different birth fields.

`census::scan::tests::repeated_scans_reuse_roots_resolved_before_an_ancestor_alias_changes` (`census/scan.rs` about 5823–5901): two full host scans to prove something about `CaptureRoots`. Call `Capture::take_roots(&roots, observer)` twice with `KernelObservation::for_test(..., Observation::Ended)` for the seeded records, retargeting the alias between the calls; keep the file-presence assertions. Check `spawn_returns_while_root_resolution_waits_and_resolves_once_across_scans` for the same cost and cut it the same way where it applies.

`shim_modes` (`crates/cargo-tile/tests/shim_modes.rs`): the slow tests are serial matrices of full shim runs. Split each cell into its own test behind one shared helper: `symlinked_capture_directories_preserve_cargo_and_target_contents` (six cells: three symlink positions by two capture paths, 774–805), `relative_home_is_omitted_from_registration_but_preserved_for_cargo` (four cells: two capture paths by two HOME values, 1159–1186), `no_terminal_returns_cargo_exit_status` (statuses 0, 1, 255 and a SIGKILL case, 1031–1057). Each new test's name states its cell. Measure `signal_to_the_shim_alone_waits_for_cargo` and cut what it waits on, or name its floor.

Each seat times its own tests with `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` and reports the before and after time of every test it changed.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — scenario entry tests, the new in-process settings walk test, the cpu scenario's cadence.
- `crates/cargo-tile/src/shim_registration/reader_scenario.py` — timestamps, `locale -a` only for locale, self-check first, fewer writers per scenario.
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — the `#[cfg(test)]` cadence on `CensusCadence`.
- `crates/cargo-tile/src/constants.rs` — named numbers for that cadence.
- `crates/cargo-tile/src/shim_registration/wire.rs` — one installation per test, no standalone Python wall-time calls.
- `crates/cargo-tile/src/census/scan.rs` — the two census tests only.
- `crates/cargo-tile/tests/shim_modes.rs` — one test per matrix cell.

**Seats:** 2 writers — the work splits by file group: the reader scenarios, and every other slow test. No tester: the phase is itself test code.
- `impl` — `crates/cargo-tile/src/shim_registration/reader_scenarios.rs`, `crates/cargo-tile/src/shim_registration/reader_scenario.py`, `crates/cargo-tile/src/census/invocation_cpu_accounting.rs`; hub: `crates/cargo-tile/src/constants.rs` (the cadence constants)
- `test` — opens as impl: `crates/cargo-tile/src/shim_registration/wire.rs`, `crates/cargo-tile/src/census/scan.rs`, `crates/cargo-tile/tests/shim_modes.rs`

**Constraints from prior phases:**

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` green, with no test at or over one second in its output and each test named in the Spec at or under about half a second (a test over that is named with what sets its floor); `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green; every test that was split or reshaped is listed with what it still proves.

### Phase 2 — Whole pids and ordered constants  · status: todo

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
- **Cost:** at most ten width checks per table layout, and one solve where there was one.

Constants in name order. The rule (`~/rust/nate_style/rust/constants-file-organization.md`): "Within each section, sort constants alphabetically by name." Use plain ascending ASCII order of the identifier. Each doc comment and attribute moves with its constant. No value, type or visibility changes.

- `crates/cargo-tile/src/constants.rs`, `// running-cargo table` (lines 161–420, next section at 422). All `pub(crate) const` but the private `MANIFEST_PATH_FLAG`. The Linux constants carry `#[cfg(target_os = "linux")]` (202–214) and `CENSUS_TEST_CADENCE_DIVISOR` carries `#[cfg(test)]` (276): each attribute moves with its constant. `COMPILER_PROCESS_NAMES` uses `RUSTC_BINARY`; a forward reference is legal. `PROCESS_TREE_NOTE_LABEL`'s doc comment (399–403) says "Stands alone like the two above it": name them, `ATTRACT_NOTE_LABEL` and `FROZEN_NOTE_LABEL`. Comments that say a column stands beside or ahead of another are about `TABLE_HEADERS` and stay.
- `crates/tui_pane/src/tiles/constants.rs`, `// cell readout` (lines 33–59). Name order: `TILE_FOOT_GAP`, `TILE_FOOT_LEFT_INSET`, `TILE_NUMBER_INDENT`, `TILE_ROWS_CELL_LABEL`, `TILE_ROWS_CELL_SEPARATOR`, `TILE_ROWS_CONTENT_LABEL`, `TILE_ROWS_READOUT_HEIGHT`, `TILE_ROWS_RIGHT_INSET`, `TILE_ROWS_WIDTH_LABEL`.
- The drop-order constant this phase adds goes into the `// running-cargo table` section in name order.

**Files:**
- `crates/cargo-tile/src/render.rs` — whole-column dropping in `TableLayout::of`, the solved-width pass, blank pid cells under their text width, ancestry pids whole or left off, the tests.
- `crates/cargo-tile/src/constants.rs` — the `// running-cargo table` section in name order, the drop-order constant.
- `crates/tui_pane/src/tiles/constants.rs` — the `// cell readout` section in name order.

**Seats:** 2 writers — the work splits by file: the table code with its tests, and the two constants files. The render tests need private items of `render.rs`, so they are written with the code.
- `impl` — `crates/cargo-tile/src/render.rs`
- `test` — opens as impl: `crates/tui_pane/src/tiles/constants.rs`; hub: `crates/cargo-tile/src/constants.rs` (the re-sort, and the drop-order constant `impl` asks for)

**Constraints from prior phases:** Phase 1 may add cadence constants beside `CENSUS_TEST_CADENCE_DIVISOR` in the `// running-cargo table` section; they are sorted with the rest. Phase 1 changes no drawing code.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`, `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` and `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` green; `bash ~/.claude/scripts/delegate/verify.sh check tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` and `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` green; the width-sweep tests for both table kinds and the ancestry chain pass; both constants sections are in ascending name order with every value, type and visibility unchanged (`git diff` of each section shows moved lines and the one reworded doc comment only); the real binary beside a real build at 64 columns shows whole pids and whole headers.
