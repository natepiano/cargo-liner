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

### Phase 2 — Show the `mem` column  · status: todo

#### Work Order

**Goal:** Every table in cargo-tile, the summary's and each command's, draws a `mem` column between `cpu` and `command`, in gibibytes to one decimal place.

**Spec:**

1. **Header and indices** (`constants.rs`). `TABLE_HEADERS` becomes `[&str; 10]`: `"pid", "parent", "start", "dur", "cpu", "mem", "state", "command", "compiler", "runs"`. New `MEMORY_COLUMN: usize = 5`, documented like its neighbours; `STATE_COLUMN = 6`, `COMMAND_COLUMN = 7`, `COMPILER_COLUMN = 8`, `MANAGED_COLUMN = 9`. `mem` stands directly after `cpu`, ahead of `state` (author's call: `state` joins only while a row is blocked, and the two measurements read together). `mem` is not added to `SUMMARY_HIDDEN_COLUMNS`: both table kinds draw it. `UNAVAILABLE_MEASUREMENT`'s doc names `mem` too.
2. **Unit** (author's call, from "round to 1 decimal place"): always gibibytes, one decimal, suffix `G` — `0.0G`, `0.3G`, `12.4G`. One unit keeps the column a fixed width and lets the rows be added by eye. New constants `BYTES_PER_GIBIBYTE: u64 = 1 << 30` and `MEMORY_UNIT: &str = "G"`.
3. **Label** (`render.rs`):
   ```rust
   /// Resident bytes as gibibytes to one decimal place, rounded half up.
   fn memory_label(bytes: u64) -> String
   ```
   Integer arithmetic, no float cast: `tenths = (u128::from(bytes) * 10 + u128::from(BYTES_PER_GIBIBYTE) / 2) / u128::from(BYTES_PER_GIBIBYTE)`, then `format!("{}.{}{MEMORY_UNIT}", tenths / 10, tenths % 10)`. The cell's text is `process.memory.map(memory_label).to_string()`, which draws `--` for an unavailable reading through `Measurement`'s `Display`.
4. **Table** (`render.rs`). `fitted_constraints` observes `MEMORY_COLUMN` with the cell text's width. `process_row`'s `cells` array gains the `mem` cell directly after the `cpu` cell, in the same `muted` style. `summary_rows`, where it copies `subtree_cpu` into a promoted row's `cpu`, also copies `subtree_memory` into its `memory`. Named cost: the column takes its width plus `TABLE_COLUMN_SPACING` (7 cells at `12.4G`) from each cell's `command` column.
5. **Tests** (`render.rs` `mod tests`):
   - `TABLE_HEADERS[CPU_COLUMN + 1] == "mem"` and `TABLE_HEADERS[MEMORY_COLUMN + 1] == "state"`, and a drawn header line reads `cpu`, `mem`, `command` in that order with no blocked row;
   - `memory_label`: `0` → `0.0G`; `53_687_091` → `0.0G`; `53_687_092` → `0.1G`; `1 << 30` → `1.0G`; `13_249_974_108` → `12.3G`;
   - an unavailable reading draws `--` in the `mem` cell in both `TableKind`s, and a reading draws its label in both;
   - a promoted summary row draws its `subtree_memory`, not its own bucket;
   - every existing test still passes (`cpu_cell_text` finds `mem` as the column after `cpu`).
6. **Docs.** `README.md`: add `mem` to each sample table that shows `cpu` (three near lines 206–236) and one sentence where the columns are described: `mem` is the command's resident memory, its compilers and tests included, in gibibytes. `CHANGELOG.md`: one `### Added` line under `## [Unreleased]`.

**Files:**
- `crates/cargo-tile/src/constants.rs` — header, indices, unit constants.
- `crates/cargo-tile/src/render.rs` — `memory_label`, `fitted_constraints`, `process_row`, `summary_rows`, tests.
- `crates/cargo-tile/README.md` — sample tables and the column sentence.
- `crates/cargo-tile/CHANGELOG.md` — the Added line.

**Seats:** 2 writers — the code is `constants.rs` and `render.rs` together, with its tests in `render.rs`'s own test module; the docs are separate files.
- `impl` — `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/render.rs`.
- `test` — opens as impl: `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md`.

**Constraints from prior phases:** Phase 1 put `memory: Measurement<u64>` and `subtree_memory: Measurement<u64>` (resident bytes) on `CargoProcess`, directly after `subtree_cpu`. A group lead's `memory` is its whole group's total; every other row's is its own bucket. Readings are already held to the cpu reporting interval on the worker, so render formats what it is given. `aggregate_cpu` is now `aggregate`. The two test row builders in `render.rs` (`CargoProcess` literals near 2437 and 2766) already carry both fields as `Measurement::Unavailable(MeasurementAbsence::Unproven)`; a render test sets `row.process.memory` to the reading it needs.

**Acceptance gate:** Build, Test and Lint from the Delegation Context are green with the Spec item 5 tests. Running `cargo-tile` beside a build shows `mem` between `cpu` and `command` in the summary and in the build's own cell, as `N.NG`.

### Phase 3 — Memory total on the summary  · status: todo

#### Work Order

**Goal:** The summary cell's bottom row reads `mem 12.3G` at its left end: the memory of every running command together.

**Spec:**

The bottom interior row of every cell is the grid's readout row (`content rows: N  r/c: h/w`, right-aligned, drawn by `tui_pane`). Its left end is empty, and that is where the total goes (author's reading of "bottom left of the summary cell"). The grid owns that row, so the grid draws the text and the app supplies it.

1. **`tui_pane` trait** (`tiles/draw.rs`). `TileCells` gains, with a default so `cargo-handler` is untouched:
   ```rust
   /// Text written at the left end of the summary cell's readout row.
   /// None unless the app has some.
   fn summary_foot(&self) -> Option<Line<'static>> { None }
   ```
2. **Drawing.** `draw_tile_cell` keeps its public signature. Its body moves to a private `draw_cell(buffer, content, inner, rows, measured_at, foot: Option<&Line<'static>>, draw)`, which `draw_tile_cell` calls with `None`. `draw_tile_grid` calls `draw_cell` directly, passing `cells.summary_foot()` for the `TileContent::Summary` placement and `None` for every other. It is inside the `contents == TileGridContents::Shown` branch, so hidden contents draw no foot.
   - The foot is drawn only where the readout row exists: `rows_readout_height(inner.width) > 0` and `inner.height >= TILE_ROWS_READOUT_HEIGHT`. Its row is the readout's (`inner.bottom() - TILE_ROWS_READOUT_HEIGHT`), its x is `inner.x + TILE_FOOT_LEFT_INSET`, and its width is the line's width capped at `inner.width - TILE_FOOT_LEFT_INSET`.
   - The foot has the row first (author's call: the total is what the user asked for; the readout is a layout diagnostic). With a foot drawn, the readout is drawn only when its whole line fits in `inner.width - TILE_ROWS_RIGHT_INSET - TILE_FOOT_LEFT_INSET - foot width - TILE_FOOT_GAP`; otherwise it is left off that frame. With no foot, the readout is drawn exactly as today.
   - New constants in `tiles/constants.rs`: `TILE_FOOT_LEFT_INSET: u16 = 1`, `TILE_FOOT_GAP: u16 = 2`.
   - The readout row is already counted in every demand (`add_readout_rows`), so the foot adds no rows and changes no layout.
3. **`tui_pane` tests** (`draw.rs` `mod tests`, with a stub `TileCells`):
   - a foot is written from `inner.x + 1` on the last interior row of the summary cell, and on no other cell;
   - in a wide cell the readout still ends one column short of the right border;
   - in a cell too narrow for both, the foot is drawn and no readout text is;
   - with no foot the buffer equals the one `draw_tile_cell` draws today;
   - `TileGridContents::Hidden` draws no foot.
4. **cargo-tile** (`render.rs`). `Cells` implements `summary_foot` as `memory_total(self.roster)`:
   ```rust
   /// The summary's memory total: every running command's group total.
   fn memory_total(roster: &Roster) -> Option<Line<'static>>
   ```
   - It reads `group.lead.process.memory` for each group in `roster.groups()` whose lead is not `is_ended()`. A lead's `memory` is already its whole group's total, a driver the summary hides included, so nothing is counted twice and nothing is left out.
   - No running group → `None`.
   - Otherwise the line is `Span::styled(SUMMARY_MEMORY_LABEL, label_color())` then the value in `text_default()`. The value is `memory_label(sum of the readings)`. When at least one running group has no reading the value ends in `PARTIAL_TOTAL_MARK`, so `12.3G+` reads "at least" (author's call: a total is never shown as whole when it is not, and one unreadable command does not blank it). When no running group has a reading the value is `UNAVAILABLE_MEASUREMENT`.
   - New constants in `constants.rs`: `SUMMARY_MEMORY_LABEL: &str = "mem "`, `PARTIAL_TOTAL_MARK: &str = "+"`.
5. **cargo-tile tests** (`render.rs` `mod tests`): two running groups of `1 << 30` and `3 << 29` bytes give `mem 2.5G`; an ended group is left out; one reading and one unavailable give `mem 1.0G+`; only unavailable gives `mem --`; an empty roster gives `None`.
6. **Docs.** `crates/tui_pane/CHANGELOG.md`: an `### Added` section under `## [Unreleased]` naming `TileCells::summary_foot`. `crates/cargo-tile/CHANGELOG.md`: one Added line. `crates/cargo-tile/README.md`: one sentence beside the Phase 2 column sentence — the summary's bottom row totals the memory of every running command, with `+` when a command cannot be read.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — `summary_foot`, `draw_cell`, foot and readout placement, tests.
- `crates/tui_pane/src/tiles/constants.rs` — the two foot constants.
- `crates/tui_pane/CHANGELOG.md` — the Added entry.
- `crates/cargo-tile/src/render.rs` — `summary_foot`, `memory_total`, tests.
- `crates/cargo-tile/src/constants.rs` — the label and the mark.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md` — the total.

**Seats:** 2 writers — split by crate; the trait method's signature above is all the cargo-tile writer needs from `tui_pane`.
- `impl` — `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/constants.rs`, `crates/tui_pane/CHANGELOG.md`.
- `test` — opens as impl: `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md`.

**Constraints from prior phases:** Phase 1: `CargoProcess.memory` is `Measurement<u64>` resident bytes, and a group lead's is the whole group's total. Phase 2: `render.rs` has `memory_label(bytes: u64) -> String` (gibibytes, one decimal, `G`), and `constants.rs` has `MEMORY_UNIT` and `BYTES_PER_GIBIBYTE`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane`, Build, Test and Lint for `cargo-tile`, and `bash ~/.claude/scripts/delegate/verify.sh check cargo-handler` and `... check cargo-port` are green, with the Spec item 3 and 5 tests. Running `cargo-tile` beside a build shows `mem N.NG` at the bottom left of the summary cell and `content rows:` still at its bottom right; with nothing running the bottom left is empty.
