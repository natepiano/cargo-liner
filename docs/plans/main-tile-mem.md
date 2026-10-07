# tile-mem

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-tile shows each running command's memory in a `mem` column and a memory total at the bottom left of the summary cell.

> **Production: remove-running-section** — unit `tile-mem-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-07 12:46 PDT

it's purpose is to provide the memory usage of the running command as a column - on cargo-tile - it should round to 1 decimal place. mem can be its call header and go between cpu and command. i want a mem total on the summary on the bottom left of the summary cell

## Delegation Context

- **Project:** `cargo-tile` — a terminal grid of every running cargo command, one cell per command plus a summary cell. Its grid is drawn by `tui_pane`, which `cargo-handler` and `cargo-port` also build on.
- **Project started:** 2026-10-07T19:56:19.784+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-tile-mem`, branch `main-tile-mem`. Every seat works only here. Set by the showrunner (production doc, Units table).
- **Stack:** Rust workspace, ratatui 0.30.2, sysinfo 0.39.6.
- **Layout:**
  - `crates/cargo-tile/src/census/` — the scan worker: reads processes, attributes them to cargo invocations, builds rows.
  - `crates/cargo-tile/src/render.rs`, `constants.rs`, `roster.rs` — the table, its constants, and the rows kept between scans.
  - `crates/tui_pane/src/tiles/` — the tile grid both apps draw through.
- **Key files:**
  - `crates/cargo-tile/src/census/scan.rs` — `CargoProcess` (242), `scan` (475), `process_discovery_refresh_kind` (554), `ProcessObservation` (602), `Census` fields (796) and `take` (842), `attribute_with` (1001), `attribute_cpu_with` (1042), `cpu_assignment` (1142), `groups` (1960), `group` (2170), `aggregate_cpu` (2384), `subtree_cpu` (2402), `row` (2475), `registration_row` (2602), `cpu_label` (2637), test fixture `CensusSequence` (2735).
  - `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — `Measurement<T>` (76), `InvocationMeasurements` (129), `InvocationCpuAccounting` (229), `settle` (685), `is_due` (729).
  - `crates/cargo-tile/src/census/mod.rs` — module list; test modules are declared here.
  - `crates/cargo-tile/src/census/summary_totals_tests.rs` — the model for a fixture-driven census test (`SummaryTree`, `CensusSequence::sample_cpu`).
  - `crates/cargo-tile/src/constants.rs` — `UNAVAILABLE_MEASUREMENT` (287), `TABLE_HEADERS` and the `*_COLUMN` indices (321–360), `SUMMARY_HIDDEN_COLUMNS` (376).
  - `crates/cargo-tile/src/render.rs` — `Cells` and its `TileCells` impl (261–296), `summary_rows` (610), `fitted_constraints` (1583), `process_row` (1652), `visible_columns` (1784), tests from 2075 (`cpu_cell_text` 2281).
  - `crates/cargo-tile/src/roster.rs` — `TrackedRow` (`process`, `is_ended()`), `TrackedGroup` (`lead`, `rows()`), `Roster::groups()`.
  - `crates/tui_pane/src/tiles/draw.rs` — `TileCells` (65), `draw_tile_grid` (119), `draw_tile_cell` (211), `draw_rows_readout` (290), `readout_area` (345), tests at the end of the file.
  - `crates/tui_pane/src/tiles/constants.rs` — the readout constants (`TILE_ROWS_RIGHT_INSET`, `TILE_ROWS_READOUT_HEIGHT`).
  - `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — user docs; each changelog has an `## [Unreleased]` section.
- **Test lanes:** `cargo-tile` — `crates/cargo-tile/tests/` (`unit_tests.rs` compiles every `src` module with its `#[cfg(test)]` tests, so a test module under `src/` runs in that lane). `tui_pane` — `crates/tui_pane/tests/`; the tile tests live in-module in `draw.rs` because they need private items. `cargo-handler` — none.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - A reading is never invented. A value the scan cannot establish is `Measurement::Unavailable` and draws as `UNAVAILABLE_MEASUREMENT` (`--`). A group total with an unavailable member is unavailable, never a partial sum (existing rule, `Measurement`'s `Add`).
  - Every literal the code names lives in the crate's `constants.rs` (existing crate convention).
  - No test takes seconds. Census and render tests are pure and run on fixtures (user rule, memory `no-multi-second-tests`).
  - `tui_pane` changes are additive: no existing public signature changes (author's call, so `cargo-handler` and `cargo-port` compile untouched).
  - Format with `cargo +nightly fmt` only (user rule, memory `rustfmt-nightly-only`).
  - No UX guide covers cargo-tile (production doc, Production rules), so no UX check lines.

## Phases

### Phase 1 — Measure each command's memory  · status: done

#### As-built

- The census reads resident memory for every process: `process_discovery_refresh_kind()` asks sysinfo for it, `ProcessObservation.memory: u64` carries it, and `Census.memory: HashMap<Pid, u64>` holds it. `Census::attribute_memory(&self, detached: &HashMap<Pid, Pid>) -> HashMap<Pid, Measurement<u64>>` attributes it per cargo command the way cpu is attributed, compilers started through sccache included.
- `CargoProcess.memory: Measurement<u64>` and `CargoProcess.subtree_memory: Measurement<u64>` hold resident bytes and sit directly after `subtree_cpu`. A group lead's `memory` is its whole group's total; every other row's is its own bucket. `subtree_memory` is the row's own bucket plus its nested cargo descendants. A registration-only row has both as `Unavailable(Unproven)`.
- Readings change on the same scans as the cpu reading. `InvocationCpuAccounting::report_memory(&mut self, sampled, cargo: &[InvocationId], due: bool)` keeps a held reading until `due`, replaces it at once when the held value or the new sample is unavailable, and drops commands that have left. `attribute_with` reads `is_due` before `settle`.
- `aggregate` (`pub(crate)`) and `subtree_totals` in `census/scan.rs` are generic over `T: Copy + Default + Add<Output = T>` and serve cpu and memory.
- Nothing drawn differs: `TABLE_HEADERS` is unchanged.

**Files:**
- `crates/cargo-tile/src/census/scan.rs` — the memory read, attribution, aggregation, the two row fields, and the test fixture `CensusSequence::attribute_memory`.
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — `InvocationMeasurements.memory`, the held readings (`reported_memory`), and the detached-compiler map shared by cpu and memory (`detached`).
- `crates/cargo-tile/src/census/memory_tests.rs` — eight fixture tests, declared in `census/mod.rs`.

**Binds later work:** `CargoProcess.memory` and `subtree_memory` are `Measurement<u64>` in bytes, already held to the cpu reporting interval, so the `mem` column and the summary total format what they are given. Every `CargoProcess` literal carries both fields. `aggregate_cpu` no longer exists; `aggregate` replaces it.

**Gotchas:**
- A cargo whose own process reads 0 bytes, or is absent from the sample, is `Unavailable(ReadFailed)`, and so is its group total.
- `row` takes cpu and memory together as one private `ResourceUse` value, which keeps it inside the crate's argument-count lint.

**Ruled out:** revalidating a process's lifetime for memory when a pid is reused between the two refreshes of one scan: too unlikely to earn mechanism.

### Phase 2 — Show the `mem` column  · status: done

#### As-built

- Every process table draws a `mem` column directly after `cpu`. `TABLE_HEADERS` is `pid`, `parent`, `start`, `dur`, `cpu`, `mem`, `state`, `command`, `compiler`, `runs`, with `MEMORY_COLUMN = 5`, `STATE_COLUMN = 6`, `COMMAND_COLUMN = 7`, `COMPILER_COLUMN = 8`, `MANAGED_COLUMN = 9`. Both table kinds draw `mem`; `state` joins only while a row is blocked.
- A cell is `process.memory.map(memory_label)`. `memory_label(bytes: u64) -> String` gives gibibytes to one decimal, rounded half up in integer arithmetic, with `MEMORY_UNIT` (`G`): `0.0G`, `0.3G`, `12.4G`. One unit keeps the column a fixed width. A row with no reading draws `UNAVAILABLE_MEASUREMENT` (`--`). A row promoted to the summary shows its group's `subtree_memory`.
- Each table chooses its own gap. `table_column_spacing(width, &constraints) -> u16` returns `TABLE_COLUMN_SPACING` (2) when the fitted columns fit the drawn width and `TIGHT_TABLE_COLUMN_SPACING` (1) when they do not. `TableLayout.column_spacing` carries the result to both table builders and to `command_column_width`, so the command column wraps at the width that table gives it.
- Render tests pin the column order, the label's rounding, the read and unread cells in both table kinds, the tight 65-wide cell (whole headers and whole parent pids at one-cell gaps), the fitting 80-wide cell (two-cell gaps), and command wrapping with no text lost.

**Files:**
- `crates/cargo-tile/src/render.rs` — `memory_label`, the memory cell, `table_column_spacing`, `TableLayout.column_spacing`, the render tests.
- `crates/cargo-tile/src/constants.rs` — the `mem` header, the column indices, `MEMORY_UNIT`, `BYTES_PER_GIBIBYTE`, `TIGHT_TABLE_COLUMN_SPACING`.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md` — the column, with sample tables spaced as the app draws them.
- `crates/cargo-tile/src/census/scan_cpu_scenario_tests.rs`, `crates/cargo-tile/src/shim_registration/app_scenarios.rs`, `reader_scenario.py`, `rows_readout.rs` — each row's count of `--` cells includes the memory cell.

**Binds later work:** `memory_label(bytes: u64) -> String`, `MEMORY_UNIT`, `BYTES_PER_GIBIBYTE` and `UNAVAILABLE_MEASUREMENT` format every memory value. The per-table gap rule stays as it is. `render::draw_cell_for_test` draws one cell through `tui_pane::draw_tile_cell`; only `render::draw` reaches `tui_pane::draw_tile_grid`.

**Gotchas:** Tests outside `render.rs` count a row's `--` cells, so a new measurement column changes them. The grid picks its column count from the terminal width: tight command cells appear at 126 columns, two wide cells at 200. A table too narrow even at one-cell gaps still cuts columns: a 31-wide summary cuts `start` to `sta`.

**Ruled out:** leaving `parent` out of a cell too narrow for it (the row loses whose child it is); adding `mem` to the columns the summary hides.

### Phase 3 — Memory total on the summary  · status: done

#### As-built

- The summary cell's bottom interior row carries the memory of every running command together at its left end: the label `mem ` then `memory_label`'s figure (`mem 2.5G`). The `content rows:` readout stays at the right end of the same row.
- `tui_pane` owns the row and the app supplies the text. Public `SummaryFoot::{Empty, Text(Line<'static>)}` and the defaulted trait method `TileCells::summary_foot(&self) -> SummaryFoot` (default `Empty`) are re-exported from `tiles/mod.rs` and `lib.rs`. `draw_tile_grid` asks for the foot only for the `TileContent::Summary` placement and only while contents are shown; `draw_tile_cell` keeps its signature and passes `Empty` to the private `draw_cell`.
- Row rule: the foot starts `TILE_FOOT_LEFT_INSET` (1) in from the cell's left border, capped at the cell's width. The readout is drawn only when its whole line fits after the foot plus `TILE_FOOT_GAP` (2); otherwise it is left off that frame. With `Empty` the readout draws as before. The readout row was already counted in every demand, so the foot adds no rows.
- cargo-tile: `SummaryMemoryTotal::{NoRunningCommands, Complete(u64), AtLeast(u64), NoReadableCommands}` and `memory_total(roster: &Roster) -> SummaryMemoryTotal` in `render.rs`. It reads `group.lead.process.memory` for each group whose lead has not ended and adds the readings with `u64::saturating_add`. `foot()` gives nothing, `mem N.NG`, `mem N.NG+` (`PARTIAL_TOTAL_MARK`: at least one running command could not be read) and `mem --`. `Cells::summary_foot` is `memory_total(self.roster).foot()`.
- Tests: `summary_foot_*`, `draw_tile_cell_draws_only_the_readout_on_its_foot_row` and `hidden_grid_draws_no_summary_foot` in `tiles/draw.rs`; `summary_memory_total_*` in `render.rs`; the whole-frame `summary_memory_foot_reaches_the_grid_readout_row` (120 columns) and `narrow_summary_keeps_memory_total_and_omits_rows_readout` (64 columns) in `app_scenarios.rs`.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — `SummaryFoot`, `TileCells::summary_foot`, `draw_cell`, `draw_summary_foot`, `draw_rows_readout_after_foot`, their tests.
- `crates/tui_pane/src/tiles/constants.rs` — `TILE_FOOT_LEFT_INSET`, `TILE_FOOT_GAP`.
- `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — the `SummaryFoot` re-exports.
- `crates/cargo-tile/src/render.rs` — `SummaryMemoryTotal`, `memory_total`, `Cells::summary_foot`, the state tests.
- `crates/cargo-tile/src/constants.rs` — `SUMMARY_MEMORY_LABEL`, `PARTIAL_TOTAL_MARK`.
- `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — the two whole-frame tests and `summary_memory_foot_row`.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — the total and the new API.

**Gotchas:** `memory_total` never uses `Measurement`'s `Add`, which makes a whole sum unavailable when one operand is; an unreadable command gives `AtLeast`. A lead's `memory` is already its whole group's total, so nothing is counted twice. A whole-frame test must cut a screen line at the summary cell's own borders (`│`, `├`, `┤`, `┼`) before asserting, because the line also holds the cells beside it. `cargo-handler` and `cargo-port` keep the default foot and compile unchanged.

**Ruled out:** a bare `Option` for the foot or the total (named states say which case holds); clipping the readout beside the total (it is whole or absent); a total on any cell but the summary.

