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

#### Work Order

**Goal:** Every `CargoProcess` the scan produces carries the resident memory of its command, attributed the way `cpu` is. Nothing on screen changes.

**Spec:**

Memory is resident set size in bytes, read from sysinfo's `Process::memory()` (author's call: resident, not virtual).

1. **Read.** `process_discovery_refresh_kind()` adds `.with_memory()`. Named cost: on Linux sysinfo then reads `/proc/<pid>/statm` once per process per scan, about 2 ms per scan at 900 processes (measured on natedev 2026-10-07); on macOS it is the same `proc_pidinfo` call `with_cpu()` already makes.
2. **Observe.** `ProcessObservation` gains `pub(super) memory: u64`, set from `process.memory()` in `metadata` and `0` in the test constructor `cargo`. `Census` gains `memory: HashMap<Pid, u64>`, filled in `take` for every process.
3. **Attribute.** New `Census` method:
   ```rust
   /// Resident bytes per row-holding cargo: its own process and every process assigned to it.
   fn attribute_memory(&self, detached: &HashMap<Pid, Pid>) -> HashMap<Pid, Measurement<u64>>
   ```
   For each `(pid, bytes)` in `self.memory`, `self.cpu_assignment(pid, detached)` names the owner: `Direct(owner)` and `Detached { owner, .. }` add `bytes` to the owner's bucket (saturating); `Unassigned` adds nothing. Buckets are disjoint: a nested cargo keeps its own bucket and its parent's does not include it, the same as `cpu`. The result has one entry per pid in `self.cargo`: `Unavailable(MeasurementAbsence::ReadFailed)` when that pid's own process is missing from `self.memory` or reads `0` (a live process always has resident pages; macOS reads `0` for a process another user owns), otherwise `Reading(bucket)`. Named cost: one more `cpu_assignment` walk per process per scan.
4. **Share the detached map.** `attribute_cpu_with` already builds `detached` (compiler pid → owner pid, how sccache-started compilers are credited). It stores that map in a new field `InvocationCpuAccounting::detached: HashMap<Pid, Pid>`, replaced every scan, so its signature and its tests stay as they are. `attribute_with` passes `&smoothing.detached` to `attribute_memory` after the cpu pass.
5. **Hold.** The `mem` reading changes on the same scans the `cpu` reading does. `InvocationCpuAccounting` gains `reported_memory: HashMap<InvocationId, Measurement<u64>>` and:
   ```rust
   /// What the `mem` column carries: replaced when a cpu reading falls due.
   pub(super) fn report_memory(
       &mut self,
       sampled: &HashMap<InvocationId, Measurement<u64>>,
       cargo: &[InvocationId],
       due: bool,
   ) -> HashMap<InvocationId, Measurement<u64>>
   ```
   It drops identities not in `cargo`. For each identity in `cargo` the sample is its entry in `sampled`, or `Unavailable(Unproven)`. The sample replaces the held value when `due`, when nothing is held, or when either the held value or the sample is `Unavailable`. It returns a clone of the held map. In `attribute_with`, read `let due = smoothing.is_due(now);` **before** `smoothing.settle(..)`, which stamps the publication. Key the samples by `self.identities`, as the cpu samples are.
6. **Carry.** `InvocationMeasurements` gains `pub(super) memory: HashMap<Pid, Measurement<u64>>`. `CargoProcess` gains, directly after `subtree_cpu`:
   ```rust
   /// Resident bytes. A group lead carries the group's total; other rows carry their own bucket.
   pub(crate) memory:         Measurement<u64>,
   /// This invocation's bucket plus every assembled cargo descendant, for the summary.
   pub(crate) subtree_memory: Measurement<u64>,
   ```
7. **Aggregate through the cpu code, not a copy of it.** `aggregate_cpu` becomes generic and is renamed `aggregate`:
   `pub(crate) fn aggregate<T: Copy + Default + Add<Output = T>>(shares: &HashMap<Pid, Measurement<T>>, members: impl Iterator<Item = Pid>) -> Measurement<T>` (the fold starts at `Measurement::Reading(T::default())`). `subtree_cpu` becomes generic the same way and is renamed `subtree_totals`. Every place `groups` (1984–2020), `group` (2192, 2220) and `row` set `cpu` or `subtree_cpu` from `attributed.cpu` also sets `memory` or `subtree_memory` from `attributed.memory`, under the same conditions and with no label step (bytes stay bytes). `row` takes the memory measurement as a parameter. `registration_row` sets both to `Unavailable(MeasurementAbsence::Unproven)`. Update the three `aggregate_cpu` call sites in `render.rs` tests (2305, 2327, 2826).
8. **Fixtures.** `CensusSequence::sample_attributed` adds `memory: census.attribute_memory(&HashMap::new())`, so a fixture's `ProcessObservation.memory` reaches its rows. `CensusSequence::assemble` and `no_measurements` add `memory: HashMap::new()`. Every `CargoProcess { .. }` literal gains both fields as `Measurement::Unavailable(MeasurementAbsence::Unproven)`: `terminal.rs` (328, 358, 395), `roster.rs` (858), `shim_registration/rows_readout.rs` (68), `render.rs` (2425, 2752), `census/scan.rs` (5444), and `census/summary_totals_tests.rs` (255) if it builds one in full.
9. **Tests**, in a new `crates/cargo-tile/src/census/memory_tests.rs`, declared `#[cfg(test)] mod memory_tests;` in `census/mod.rs`. Build processes with `ProcessObservation::cargo(pid, argv)`, set `parent` and `memory`, and run them through `CensusSequence` as `summary_totals_tests.rs` does; a non-cargo child is a `cargo` observation whose `name` is replaced.
   - a command's reading is its own bytes plus its non-cargo children's;
   - in a driver → child → nested tree the lead reads the whole group, each other row reads its own bucket, and the child's `subtree_memory` is its bucket plus the nested one;
   - a cargo whose own process reads `0` is `Unavailable`, and so is its group lead (no partial total);
   - a process under a detached compiler root is credited to the owner named in the `detached` map (call `attribute_memory` directly);
   - `report_memory` keeps a held reading while not due, takes the new one when due, and passes a first reading and an `Unavailable` sample straight through;
   - a registration-only row has no reading.

**Files:**
- `crates/cargo-tile/src/census/scan.rs` — refresh kind, `ProcessObservation.memory`, `Census.memory`, `attribute_memory`, `attribute_with`, generic `aggregate` and `subtree_totals`, `CargoProcess` fields, `row`, `registration_row`, `groups`, `group`, fixtures.
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — `InvocationMeasurements.memory`, `detached`, `reported_memory`, `report_memory`.
- `crates/cargo-tile/src/census/mod.rs` — declares `memory_tests`.
- `crates/cargo-tile/src/census/memory_tests.rs` — new; the tests above.
- `crates/cargo-tile/src/census/summary_totals_tests.rs` — literal fields, if any.
- `crates/cargo-tile/src/render.rs` — test literals and the renamed `aggregate` calls only.
- `crates/cargo-tile/src/roster.rs` — test literal.
- `crates/cargo-tile/src/terminal.rs` — test literals.
- `crates/cargo-tile/src/shim_registration/rows_readout.rs` — test literal.

**Seats:** 1 writer + 1 tester — the code is one chain through `scan.rs`, and the tests are a new file written from the Spec.
- `impl` — every file above except `memory_tests.rs`; hub: `crates/cargo-tile/src/census/mod.rs` (declares the test module).
- `test` — `crates/cargo-tile/src/census/memory_tests.rs`, from Spec items 2, 3, 5, 6 and 9.

**Constraints from prior phases:**

**Acceptance gate:** Build, Test and Lint from the Delegation Context are green, with every test in Spec item 9 present and passing. `TABLE_HEADERS` is unchanged, so no drawn cell differs from before this phase.

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

**Constraints from prior phases:** Phase 1 put `memory: Measurement<u64>` and `subtree_memory: Measurement<u64>` (resident bytes) on `CargoProcess`, directly after `subtree_cpu`. A group lead's `memory` is its whole group's total; every other row's is its own bucket. Readings are already held to the cpu reporting interval on the worker, so render formats what it is given. `aggregate_cpu` is now `aggregate`.

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
