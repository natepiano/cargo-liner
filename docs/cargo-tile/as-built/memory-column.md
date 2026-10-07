# cargo-tile `mem` column and summary memory total

## What it is

cargo-tile shows resident memory for each running command in a `mem` column after `cpu` and before `state` and `command`. The value includes processes attributed to that cargo invocation, such as compilers and tests. The summary cell shows the total for all running commands at the left of its bottom row. A trailing `+` marks a lower bound because at least one command could not be read.

## How it works

| File | Role |
| --- | --- |
| `crates/cargo-tile/src/census/scan.rs` | Reads resident memory, attributes process readings, and builds group and subtree totals. |
| `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` | Defines `Measurement<T>`, holds published memory, and applies the reporting cadence. |
| `crates/cargo-tile/src/census/mod.rs` | Declares the census modules and re-exports the census types used elsewhere. |
| `crates/cargo-tile/src/roster.rs` | Retains live and fading groups between scans. |
| `crates/cargo-tile/src/render.rs` | Formats memory, draws the `mem` column, and computes the summary total. |
| `crates/cargo-tile/src/constants.rs` | Owns the column indices, units, labels, markers, and spacing values. |
| `crates/tui_pane/src/tiles/draw.rs` | Defines the summary-foot API and draws it on the rows-readout row. |
| `crates/tui_pane/src/tiles/constants.rs` | Owns the foot and readout geometry. |
| `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` | Re-export `SummaryFoot` and `TileCells`. |

The census reads memory during its full-system discovery refresh. `process_discovery_refresh_kind()` requests CPU and memory without Linux tasks. `ProcessObservation::memory: u64` takes `Process::memory()`, and `Census::memory: HashMap<Pid, u64>` keeps each process's resident bytes.

`Census::attribute_memory(&self, detached: &HashMap<Pid, Pid>) -> HashMap<Pid, Measurement<u64>>` uses the same ownership rule as CPU. A process belongs to the nearest cargo in its ancestry, or to the cargo named by the detached-compiler map. This includes compiler trees started through sccache. The resulting invocation buckets are disjoint.

`Measurement<T>` distinguishes `Reading(T)` from `Unavailable(MeasurementAbsence)`. Its `Add` implementation propagates an unavailable operand instead of publishing a partial total. `InvocationMeasurements.memory` carries the attributed buckets into group assembly.

Memory is published on the CPU reporting cadence. `InvocationCpuAccounting::memory_report(now) -> MemoryReport` returns `MemoryReport::Replace` when the CPU reading is due and `MemoryReport::Keep` between due scans. `Census::attribute_with` obtains that state before calling `settle`, because `settle` updates the CPU publication time. It then calls:

```rust
pub(super) fn report_memory(
    &mut self,
    sampled: &HashMap<InvocationId, Measurement<u64>>,
    cargo: &[InvocationId],
    memory_report: MemoryReport,
) -> HashMap<InvocationId, Measurement<u64>>
```

`Replace` takes the new samples. `Keep` holds readable values, but an unavailable held value or unavailable new sample is replaced immediately. Recovery therefore passes through immediately too. Newly seen commands receive their first sample, and commands that have left are removed from `reported_memory`.

`aggregate<T>` and `subtree_totals<T>` in `scan.rs` serve both CPU and memory. Both require `T: Copy + Default + Add<Output = T>`. `aggregate` deduplicates PIDs and makes a missing or unavailable member unavailable. `subtree_totals` follows assembled `VisibleParent::Invocation` links, counts each bucket once, and makes cyclic or incomplete subtrees unavailable.

`CargoProcess` carries two memory fields:

| Field | Meaning |
| --- | --- |
| `memory: Measurement<u64>` | A group lead's whole-group total. On other rows, that invocation's own attributed bucket. |
| `subtree_memory: Measurement<u64>` | That row's bucket plus all assembled cargo descendants beneath it. |

Registration-only rows set both fields to `Unavailable(Unproven)`. Group assembly withholds the lead's total when any member is not a proven process row. `ResourceUse` carries CPU and memory together into the private `row` constructor.

`Roster` retains live and fading `TrackedGroup`s. `TrackedGroup::lead` carries the group total. `TrackedGroup::rows()` exposes the lead and retained children. `TrackedRow::is_ended()` lets the summary total ignore a group whose lead has finished even while its row remains visible during its fade.

`TABLE_HEADERS` is `pid`, `parent`, `start`, `dur`, `cpu`, `mem`, `state`, `command`, `compiler`, `runs`. `MEMORY_COLUMN` is 5. `process_row` formats `process.memory` through `memory_label(bytes: u64) -> String`; unavailable values display as `--`. Both command and summary tables keep the memory column. When a child row is promoted into the summary for a hidden driver, the copy uses `subtree_memory`, while the roster retains the row's own-bucket value.

Each table computes its own spacing. `table_column_spacing(width, constraints)` uses two cells when every fitted column fits and one otherwise. `TableLayout.column_spacing` feeds both ratatui tables and `command_column_width`, so command wrapping uses the width the table actually gives it.

The summary total has four named states:

```rust
enum SummaryMemoryTotal {
    NoRunningCommands,
    Complete(u64),
    AtLeast(u64),
    NoReadableCommands,
}
```

`memory_total(&Roster)` reads `group.lead.process.memory` once for each group whose lead has not ended. It adds readable groups with `u64::saturating_add`. The four outputs are no foot, `mem N.NG`, `mem N.NG+`, and `mem --`.

`tui_pane` owns the bottom row. Public `SummaryFoot::{Empty, Text(Line<'static>)}` and the defaulted `TileCells::summary_foot(&self) -> SummaryFoot` let an app supply text without changing existing implementations. cargo-tile's `Cells::summary_foot` returns `memory_total(self.roster).foot()`.

`draw_tile_grid` requests the foot only for `TileContent::Summary` and only while contents are shown. The public `draw_tile_cell` signature is unchanged and supplies `SummaryFoot::Empty`. The foot starts one cell inside the left border. The rows readout remains right-aligned on the same bottom row only when its complete line fits after the foot and a two-cell gap. Otherwise the readout is omitted for that frame. The foot adds no row because the grid already reserves the readout row.

## Invariants

- Never invent a reading. Failed or unproven memory is `Measurement::Unavailable` and displays as `--`.
- A cargo process must have a positive own resident reading before its invocation bucket is readable. Zero or absence means `Unavailable(ReadFailed)`, even when descendants have readings.
- A normal group or subtree total with an unavailable member is unavailable. It is never presented as a complete partial sum.
- Invocation buckets are disjoint. Nested cargo keeps its own bucket. A group lead totals the group once, and the summary totals group leads once.
- `memory` and `subtree_memory` remain resident bytes. Formatting belongs in `render.rs`.
- Every `CargoProcess` construction supplies both memory fields.
- `memory_report(now)` must run before `settle(..., now)`. `settle` changes the publication state used to decide `Replace` or `Keep`.
- Memory publication follows CPU's one-second reporting boundary. Unavailability and recovery still pass through immediately.
- Registration-only rows remain unavailable and withhold totals that they make unprovable.
- The summary total includes running groups only. Fading, ended groups do not contribute.
- A partial summary is explicit. Readable groups form `AtLeast`, shown with `+`. If none are readable, show `mem --`.
- The `mem` column stays after `cpu` and remains visible in both table kinds. It is not part of `SUMMARY_HIDDEN_COLUMNS`.
- The summary foot appears only in the summary cell. It shares the existing readout row and does not change row demand.
- The memory foot wins on a narrow row. The rows readout is whole or absent, never clipped beside it.
- `tui_pane` remains source-compatible for existing users. `summary_foot` has an `Empty` default, and existing public drawing signatures do not change.
- Named literals remain in each crate's `constants.rs`.
- Census and render coverage stays fixture-based and fast.

## Calibration and gotchas

- `PROCESS_POLL_MILLIS = 250`: resident memory is refreshed for the full process list four times per second.
- `CPU_REPORT_MILLIS = 1000`: readable memory reaches the table at most once per second, except for failure or recovery transitions.
- `CPU_SMOOTHING_SECONDS = 2.0`: CPU is smoothed across this window. Memory is not smoothed; it is sampled and held.
- `BYTES_PER_GIBIBYTE = 1 << 30` and `MEMORY_UNIT = "G"`. The display is gibibytes despite the short suffix.
- `memory_label` rounds to one decimal place, half up, using integer arithmetic through `u128`. Examples are `0.0G`, `0.3G`, and `12.4G`.
- `UNAVAILABLE_MEASUREMENT = "--"`, `SUMMARY_MEMORY_LABEL = "mem "`, and `PARTIAL_TOTAL_MARK = "+"`.
- `TABLE_COLUMN_SPACING = 2`; `TIGHT_TABLE_COLUMN_SPACING = 1`. A `12.4G` cell plus its normal gap takes seven columns from the command area.
- `TILE_FOOT_LEFT_INSET = 1`, `TILE_FOOT_GAP = 2`, `TILE_ROWS_RIGHT_INSET = 1`, and `TILE_ROWS_READOUT_HEIGHT = 1`.
- A 65-column command table uses one-cell gaps while retaining whole headers and parent PIDs. An 80-column fitting table keeps two-cell gaps. Those cell widths arise around 126- and 200-column terminal layouts. A table narrower than its fitted columns still truncates headers and cells.
- Adding a measurement column changes fixtures that count `--` cells. CPU, memory, compiler, and runs can all contribute that marker.
- The full-system pass now pays for resident-memory refreshes. The detailed second refresh still targets cargo, ancestors, registrations, and compiler evidence.
- `ResourceUse` groups CPU and memory for `row` so the constructor stays inside the crate's argument-count limit.
- Memory does not revalidate process lifetime between the discovery and detail refreshes. PID reuse inside that interval was judged too unlikely to justify more mechanism.
- Do not remove `parent` to save width. A narrow row would lose the identity of the cargo above it.
- Do not hide memory from the summary. It is command-level information, not invocation-only detail.
- A bare `Option` cannot express the summary's empty, complete, lower-bound, and wholly unreadable states.
- The renderer's cell-level helper uses public `draw_tile_cell`, which supplies an empty foot. Only the full grid path through `draw_tile_grid` exercises the summary foot.
- Whole-frame assertions must isolate the summary cell between its own `│`, `├`, `┤`, or `┼` borders because the same terminal line also contains neighboring cells.
- cargo-handler and cargo-port receive `SummaryFoot::Empty` from the trait default.

## Why

- **Use CPU ownership.** The displayed command should include the work it caused, not only the small cargo driver process. The detached map keeps sccache-launched compiler trees with the invocation that requested them.
- **Keep own buckets and subtree totals.** Command cells and promoted summary rows answer different questions. A command row shows its invocation. A promoted row must include nested cargo beneath it.
- **Reject silent partial totals.** Ordinary aggregation propagates unavailability because an unlabeled partial sum looks complete. The summary is different because its `+` makes the lower-bound meaning visible.
- **Total group leads only.** Each lead already carries its whole group. Summing every row would count nested work twice.
- **Hold memory with CPU.** The resource columns should not redraw on every 250 ms scan. Failure and recovery bypass the hold so stale confidence is not retained.
- **Use one fixed unit.** A fixed suffix keeps the memory column predictable. Half-up integer rounding gives the requested one-decimal value without floating-point conversion.
- **Choose spacing per table.** Identifying columns remain readable when width is tight. The command column absorbs the remaining width and wraps without losing text.
- **Put the foot in `tui_pane`.** The grid owns the bottom row and the rows readout. A named enum and default trait method add the capability without changing existing callers.
- **Keep the readout whole.** The memory total owns the left side and the rows readout owns the right. When both cannot fit, omitting the readout avoids overlap or a misleading fragment.
- **Show the total only on the summary.** It describes every running command together. No command cell has that scope.
