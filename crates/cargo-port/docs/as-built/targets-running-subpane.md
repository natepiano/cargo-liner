# Targets Running Sub-Pane As Built

Status: implemented.

This document records the current Targets pane Running sub-pane behavior. It is
an as-built summary, not the original phase plan. The process-outline and CPU
smoothing details are covered in `docs/as-built/running-process-tree.md`.

Main implementation paths:

- `src/tui/panes/targets/mod.rs`
- `src/tui/panes/targets/running_subpane.rs`
- `src/tui/running_targets/mod.rs`
- `src/tui/running_targets/app_tick.rs`
- `src/tui/panes/actions.rs`
- `src/tui/panes/pane_impls.rs`
- `src/tui/integration/framework_keymap.rs`
- `src/tui/input/mod.rs`
- `src/tui/interaction.rs`
- `tui_pane/src/layout/region.rs`

## What The Pane Shows

The Targets pane is two vertical boxes when anything is running:

```text
Stack[
  rows(targets_table, Fill).header().footer(),
  rows(running_list, Cap(80)).rule().header().lines(visible_running_lines),
]
```

When nothing is running, the pane is just the table:

```text
Stack[
  rows(targets_table, Fill).header(),
]
```

The table shows one row per target in the current Targets section. Running state
is no longer rendered inline in the table. The title groups are the current UI
groups: `Binary`, `Examples`, and `Benches`.

The Running sub-pane appears below the table whenever the global Running list is
nonempty. It is global across tracked workspaces, so it can appear even when the
selected project has no targets. The empty Targets block appears only when there
are no targets and nothing running.

## Running Box Layout

The Running box is capped at 80% of the Targets pane inner height. The table
keeps a minimum of three data rows when it has targets.

The box reserves two chrome rows:

- a divider rule titled `Running (N)` or `Running (i of N)`;
- a column-header row.

Columns are:

```text
Target | Profile | PID | CPU | MEM | Path
```

The Path column is last and left-truncated so the rightmost member path remains
visible. Child-process rows leave Profile and Path blank.

## Data Source

`App::running_targets_tick` builds process-matching slices from metadata and
visible targets, then calls `Panes::running_targets_tick`. The poller stores the
current snapshot in `RunningTargetsPoller`.

`render_targets_pane_body` reads that snapshot each render:

1. `build_running_rows(ctx.running_targets)` flattens tracked instances and
   child processes into tree order;
2. `build_running_list(rows, cargo_group, expanded_parents)` applies the
   installed-`cargo` group and outline collapse state;
3. `targets_region(table_len, running_list.len(), inner_height)` builds the box
   tree;
4. `Region::place` returns `Placed` rects for the table and Running box;
5. the table and Running renderer push row hit-test rects into
   `TargetsPane::row_rects`.

## Cursor Model

There is one `Viewport` for the whole Targets pane. Its global row index walks
the table rows first, then the Running rows.

`TargetsPane::running_cursor_pid` is the Running-row anchor:

- `None` while the cursor is in the table or on the `cargo` header;
- `Some(pid)` while the cursor is on a killable Running instance row.

During render, `sync_running_cursor` follows the anchored PID as rows reorder.
If the anchored process is gone, the cursor falls to the adjacent Running row,
then back to the table when the Running list empties. A `cargo` header row has
no PID and anchors by its stable list position.

User-driven cursor moves re-derive the anchor through
`panes::sync_running_targets_cursor`:

- navigation in `navigate_targets`;
- clicks in `interaction.rs`;
- mouse-wheel scroll in `input/mod.rs`.

## Navigation And Toggle Behavior

Targets navigation uses the shared framework navigation actions.

On table rows:

- `Enter` launches the selected target in debug mode;
- `r` launches the selected target in release mode;
- `K` is hidden and dispatch is a no-op.

On Running rows:

- `Enter` toggles the `cargo` header or an outline parent when the cursor is on
  one; otherwise it does not launch a table target;
- `Right` expands a collapsed `cargo` header or outline parent before falling
  through to row movement;
- `Left` collapses an expanded outline parent first, then the `cargo` group;
- clicking a Running row selects it, and dispatch-click toggles an expandable
  header or parent.

The keymap visibility is presentational but matches dispatch behavior:

- `Activate` and `ReleaseBuild` are visible only while the cursor is in the
  table;
- `Kill` is visible only while `running_cursor_pid().is_some()`.

## Kill Behavior

`K` kills one process only. The old "kill every instance of this target" path is
gone.

`handle_target_kill` rebuilds the current Running rows/list and calls
`resolve_kill_request`. That returns `Some(KillRequest)` only for a Running
instance row. Table rows and the `cargo` header return `None`.

The confirm action stores:

- label;
- PID;
- process create time.

The confirm body shows the label and `pid <pid> · started <age> ago`.
`execute_target_kill` calls `RunningTargetsPoller::kill(pid, create_time)`, which
refreshes that single PID and sends `SIGTERM` only if the process start time
still matches. It then calls `drop_instances` so the row disappears without
waiting for the next poll.

## Installed `cargo` Group

Installed binaries matched from the Cargo install bin directory use the
`cargo` profile. They fold under a leading `cargo` header row.

`CargoGroup` defaults to collapsed. The header count includes the installed
roots and descendants folded under the group. The header is not killable.

When expanded, installed instances appear below the header, but ordinary outline
parents under that group still obey their own collapsed-by-default state.

## Process Outline

The Running list can show nested process trees. A tracked instance or child
process with shown descendants becomes an outline parent.

Outline state lives in `TargetsPane::expanded_parents`. Missing means collapsed.
Collapsed parent rows show aggregate CPU and memory for their subtree; expanded
parent rows show their own metrics and reveal their children.

Rows use the same expand/collapse idiom as the project list:

- `Enter` toggles the row;
- `Right` expands;
- `Left` collapses, and from a child row hands the cursor to the collapsed
  parent.

## Hit Testing

`TargetsPane` records per-rendered-row `(Rect, logical_row)` values in
`row_rects`. `TargetsPane::hit_test_at` uses those rects instead of
`Viewport::pos_to_local_row`, because the pane has two boxes and the Running
box can be scrolled independently by the layout pass.

The same rects support hover styling and click selection for both table and
Running rows.

## Layout Engine Pieces

The generic layout lives in `tui_pane/src/layout/region.rs`:

- `Region::rows`, `Region::stack`, and `Region::columns`;
- `Size::Fixed`, `Size::Fill`, and `Size::Cap`;
- chrome reservation through `.rule()`, `.header()`, `.footer()`, and
  `.spacer()`;
- rendered-height overrides through `.lines()`;
- `Region::place` for rects and scroll offsets;
- `Region::locate` and `total_selectable` for highlight mapping.

The Targets pane uses only a stack of rows boxes, but the same engine also backs
the CPU and Package pane layouts.

## Current Constants

Defined in `src/tui/panes/targets/mod.rs`:

- `RUNNING_CAP_PERCENT = 80`;
- `RUNNING_CHROME = 2`;
- `TABLE_CHROME = 1`;
- `TABLE_FOOTER = 1`;
- `MIN_TABLE_ROWS = 3`.

Defined in `src/tui/panes/targets/running_subpane.rs`:

- target column cap and outline indentation widths;
- fixed Profile, PID, CPU, and MEM column widths;
- one-digit outline suffix reservation to avoid column jumps when children
  appear.
