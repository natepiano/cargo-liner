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

### Phase 3 — Memory total on the summary  · status: todo

#### Work Order

**Goal:** The summary cell's bottom row reads `mem 12.3G` at its left end: the memory of every running command together.

**Spec:**

The bottom interior row of every cell is the grid's readout row (`content rows: N  r/c: h/w`, right-aligned, drawn by `tui_pane`). Its left end is empty, and that is where the total goes (author's reading of "bottom left of the summary cell"). The grid owns that row, so the grid draws the text and the app supplies it.

1. **`tui_pane` trait** (`tiles/draw.rs`). A new public type says whether the app has text for the row, and `TileCells` gains a method with a default so `cargo-handler` is untouched:
   ```rust
   /// What the app writes at the left end of the summary cell's readout row.
   pub enum SummaryFoot {
       /// The app writes nothing there.
       Empty,
       /// The app's text, drawn from the left inset.
       Text(Line<'static>),
   }

   /// Text written at the left end of the summary cell's readout row.
   fn summary_foot(&self) -> SummaryFoot { SummaryFoot::Empty }
   ```
   `SummaryFoot` is re-exported beside `TileCells`: `pub use draw::SummaryFoot;` in `tiles/mod.rs` and `pub use tiles::SummaryFoot;` in `lib.rs`. No bare `Option` carries this state (author's call from the type design rule; the trait's existing `group_title` keeps its signature, because the plan changes no existing public signature).
2. **Drawing.** `draw_tile_cell` keeps its public signature. Its body moves to a private `draw_cell(buffer, content, inner, rows, measured_at, foot: &SummaryFoot, draw)`, which `draw_tile_cell` calls with `&SummaryFoot::Empty`. `draw_tile_grid` calls `draw_cell` directly, passing `cells.summary_foot()` for the `TileContent::Summary` placement and `SummaryFoot::Empty` for every other. It is inside the `contents == TileGridContents::Shown` branch, so hidden contents draw no foot.
   - A `SummaryFoot::Text` is drawn only where the readout row exists: `rows_readout_height(inner.width) > 0` and `inner.height >= TILE_ROWS_READOUT_HEIGHT`. Its row is the readout's (`inner.bottom() - TILE_ROWS_READOUT_HEIGHT`), its x is `inner.x + TILE_FOOT_LEFT_INSET`, and its width is the line's width capped at `inner.width - TILE_FOOT_LEFT_INSET`.
   - The foot has the row first (author's call: the total is what the user asked for; the readout is a layout diagnostic). With a `Text` foot drawn, the readout is drawn only when its whole line fits in `inner.width - TILE_ROWS_RIGHT_INSET - TILE_FOOT_LEFT_INSET - foot width - TILE_FOOT_GAP`; otherwise it is left off that frame. With `SummaryFoot::Empty`, the readout is drawn exactly as today.
   - New constants in `tiles/constants.rs`: `TILE_FOOT_LEFT_INSET: u16 = 1`, `TILE_FOOT_GAP: u16 = 2`.
   - The readout row is already counted in every demand (`add_readout_rows`), so the foot adds no rows and changes no layout.
3. **`tui_pane` tests** (`draw.rs` `mod tests`, with a stub `TileCells`):
   - a foot is written from `inner.x + 1` on the last interior row of the summary cell, and on no other cell;
   - in a wide cell the readout still ends one column short of the right border;
   - in a cell too narrow for both, the foot is drawn and no readout text is;
   - with `SummaryFoot::Empty` the buffer equals the one `draw_tile_cell` draws today;
   - `TileGridContents::Hidden` draws no foot.
4. **cargo-tile** (`render.rs`). The total is a type of its own, so each state has a name:
   ```rust
   /// The memory of every running command together, as far as it can be read.
   enum SummaryMemoryTotal {
       /// Nothing is running, so there is no total.
       NoRunningCommands,
       /// Every running command was read; the bytes are the whole total.
       Complete(u64),
       /// At least one running command could not be read; the bytes are the rest.
       AtLeast(u64),
       /// Commands are running and none could be read.
       NoReadableCommands,
   }

   /// The summary's memory total: every running command's group total.
   fn memory_total(roster: &Roster) -> SummaryMemoryTotal
   ```
   - `memory_total` reads `group.lead.process.memory` for each group in `roster.groups()` whose lead is not `is_ended()`. A lead's `memory` is already its whole group's total, a driver the summary hides included, so nothing is counted twice and nothing is left out.
   - It adds the readings itself, with `u64::saturating_add`, and counts the unreadable groups beside them. It never uses `Measurement`'s `Add`: that makes the whole sum unavailable when one operand is, which is the opposite of `AtLeast`.
   - `SummaryMemoryTotal::foot(&self) -> SummaryFoot` draws it. `NoRunningCommands` is `SummaryFoot::Empty`. Every other state is `SummaryFoot::Text` of `Span::styled(SUMMARY_MEMORY_LABEL, label_color())` then the value in `text_default()`: `Complete(bytes)` is `memory_label(bytes)`; `AtLeast(bytes)` is `memory_label(bytes)` followed by `PARTIAL_TOTAL_MARK`, so `12.3G+` reads "at least" (author's call: a total is never shown as whole when it is not, and one unreadable command does not blank it); `NoReadableCommands` is `UNAVAILABLE_MEASUREMENT`.
   - `Cells` implements `summary_foot` as `memory_total(self.roster).foot()`.
   - New constants in `constants.rs`: `SUMMARY_MEMORY_LABEL: &str = "mem "`, `PARTIAL_TOTAL_MARK: &str = "+"`.
5. **cargo-tile tests.**
   - `render.rs` `mod tests`, on the state and its text: two running groups of `1 << 30` and `3 << 29` bytes give `Complete` and `mem 2.5G`; an ended group is left out; one reading and one unavailable give `AtLeast` and `mem 1.0G+`; only unavailable gives `NoReadableCommands` and `mem --`; an empty roster gives `NoRunningCommands` and `SummaryFoot::Empty`.
   - `shim_registration/app_scenarios.rs`, through its whole-frame `draw(terminal, app)` helper, because only `render::draw` reaches `tui_pane::draw_tile_grid`: `summary_memory_foot_reaches_the_grid_readout_row` (the summary's bottom interior row starts with `mem ` and still ends with the `content rows:` readout) and `narrow_summary_keeps_memory_total_and_omits_rows_readout` (in a frame whose summary is too narrow for both, the row holds the total and no readout text).
6. **Docs.** `crates/tui_pane/CHANGELOG.md`: an `### Added` section under `## [Unreleased]` naming `SummaryFoot` and `TileCells::summary_foot`. `crates/cargo-tile/CHANGELOG.md`: one Added line. `crates/cargo-tile/README.md`: one sentence beside the Phase 2 column sentence — the summary's bottom row totals the memory of every running command, with `+` when a command cannot be read.

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — `SummaryFoot`, `summary_foot`, `draw_cell`, foot and readout placement, tests.
- `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — the `SummaryFoot` re-exports.
- `crates/tui_pane/src/tiles/constants.rs` — the two foot constants.
- `crates/tui_pane/CHANGELOG.md` — the Added entry.
- `crates/cargo-tile/src/render.rs` — `SummaryMemoryTotal`, `memory_total`, `summary_foot`, tests.
- `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — the two whole-frame tests.
- `crates/cargo-tile/src/constants.rs` — the label and the mark.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md` — the total.

**Seats:** 2 writers — split by crate; `SummaryFoot` and the trait method's signature above are all the cargo-tile writer needs from `tui_pane`, and `SummaryMemoryTotal` with its state tests needs nothing from it.
- `impl` — `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/constants.rs`, `crates/tui_pane/CHANGELOG.md`; hub: `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs`.
- `test` — opens as impl: `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/shim_registration/app_scenarios.rs`, `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md`.

**Constraints from prior phases:** Phase 1: `CargoProcess.memory` is `Measurement<u64>` resident bytes, and a group lead's is the whole group's total. Phase 2: `render.rs` has `memory_label(bytes: u64) -> String` (gibibytes, one decimal, `G`), and `constants.rs` has `MEMORY_UNIT` and `BYTES_PER_GIBIBYTE`. Phase 2 also made each table choose its own gap: `table_column_spacing` in `render.rs` returns two cells when the fitted columns fit and `TIGHT_TABLE_COLUMN_SPACING` (one) when they do not, and `TableLayout.column_spacing` carries it; this phase leaves that rule as it is. `render::draw_cell_for_test` draws one cell through `tui_pane::draw_tile_cell`, which passes no foot, so no existing test sees the summary's bottom row through the grid and none is expected to change; if one fails only because of the total, the cargo-tile writer owns it. Only `cargo-handler` and `cargo-tile` implement `TileCells`; `cargo-port` only depends on `tui_pane`. Where things are in `crates/cargo-tile/src/render.rs` after Phase 2: `draw` 203, `Cells` and its `TileCells` impl 265–300, `draw_cell_for_test` 511, `summary_rows` 614, `fitted_constraints` 1598, `process_row` 1690, `visible_columns` 1831, `memory_label` 2008, `mod tests` 2129.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane`, Build, Test and Lint for `cargo-tile`, and `bash ~/.claude/scripts/delegate/verify.sh check cargo-handler` and `... check cargo-port` are green, with the Spec item 3 and 5 tests. Running `cargo-tile` beside a build shows `mem N.NG` at the bottom left of the summary cell and `content rows:` still at its bottom right; with nothing running the bottom left is empty. The shots show four states: a whole total, a `+` total (a command that cannot be read), nothing running, and a summary too narrow for both the total and the readout (a 64-column terminal).
