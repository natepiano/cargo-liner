# Summary depth

## What it is

The summary is cell 1 of the tile grid. Sometimes its contents do not fit the rows its column gives it, even after every idle cell in that column has lent it their spare rows. It then counts as 2, 3, ... cells of column 0 (its **summary depth** `k`), and every other cell moves one position on for each cell it takes. Once fewer cells hold it, it gives cells back. At `k = 1` the arrangement is the grid without depth. Depth has no setting and is always on. The grid lives in `tui_pane`, so cargo-tile (command cells) and cargo-handler (agent cells) behave the same.

## How it works

Every depth item is private to `crates/tui_pane/src/tiles/grid.rs`.

| Item | Role |
| --- | --- |
| `HeldCellLayout::summary_depth` | Positions the summary covers in the drawn layout, its own included. `rows` stays one entry per logical cell, summary first. |
| `HeldCellLayout::summary_span` | Columns the summary asks to cover, taken from `wanted_span`. |
| `Step::depth` | Depth a queued step leaves. |
| `TileGrid::depth`, `target_depth()` | `depth` is the depth `slots` is laid out at. `target_depth()` is the last pending step's depth, else `self.depth`. |
| `summary_depth(area, wants, growth, settings) -> usize` | The depth rule. |
| `summary_share(wants, height, depth, growth, floor) -> u16` | Rows column 0 gives the summary at `depth`. |
| `enum ColumnHead { Summary(usize), Cell }` | What heads a column handed to `shares`. |
| `shares(wants, height, head, focused, floor)` | Divides one column. |
| `fits(area, count, growth, settings) -> bool` | Whether a `count`-position grid keeps every position at `MIN_TILE_WIDTH` by `MIN_TILE_HEIGHT`. |
| `column_layout(count, held_depth, growth) -> ColumnLayout { widths, summary_depth }` | The one depth resolver. |
| `Grid { depth, span, widths, .. }` | One laid-out grid: resolved depth, drawn span, column heights. |
| `queue`, `queue_with_depth`, `queue_steps` | Queue arrangement steps paired with a depth. |

### Each frame

`draw_tile_grid` (`crates/tui_pane/src/tiles/draw.rs`) calls these in order:

1. `set_layout`
2. `content_widths`: each cell's interior width at the drawn layout
3. the app's `TileCells::demands`: asks measured at those widths
4. `add_readout_rows`
5. `sync`: decides the arrangement and the depth
6. `placements`

cargo-tile's `tile_demands` (`crates/cargo-tile/src/render.rs`) measures the summary's `table_height` at the summary's entry in `content_widths`. That entry is the interior of the rect `Grid::new` gives the summary, widening included. cargo-handler's summary ask does not depend on width.

The summary foot is requested only while the summary contents are shown. It shares the readout row already added in step 4, so it changes neither the summary's ask nor its depth.

### The depth rule

`sync` calls `summary_depth` with fresh asks: `cell_wants` over the demands just measured, for the arrangement the scan is headed to. Starting from `k = 1`, it returns the first `k` whose `summary_share` over the full `area.height` reaches the summary's ask.

It stops at the current `k`, and the summary is clipped, when either:

- `k + 1` would exceed column 0's height at that depth (`columns(count + k, growth)[0]`), or
- `count + k` positions would fail `fits`.

`summary_share` divides column 0 at depth `k` (the summary plus the cells under it) with `ColumnFocus::Outside`, so focus has no say. Column 0 is never under a widened summary, so its full height is the measure.

### Division

`Grid::new` heads column 0 with `ColumnHead::Summary(k)` and every other column with `ColumnHead::Cell`. Under `Summary(k)`, `shares`:

- weights the summary `k` and every other piece 1 in the even split (`apportion`);
- raises each ask to `floor` (`MIN_TILE_HEIGHT`) and caps it at the column height;
- fills the summary's shortfall from the spare rows of cells asking less than their share, before the focus ring's shortfall. The other short cells then divide what is left in proportion.

So the summary deepens only when it is still short after every spare row in column 0 has gone to it. `TileGrid::fits` applies `fits` to the last frame's area.

### Queuing

`sync` builds its arrangement steps (closes, admissions, reorders). It computes the wanted depth once from the final arrangement's asks and calls `queue_with_depth(steps, wanted)`. It adds a bare re-divide step only when all of these hold:

- there are no steps;
- `wanted == target_depth()`;
- the grid is settled;
- `settled_held() != self.held`.

`queue(steps)` is used by `+` and `-` (`add`, `remove`). It works out the wanted depth from the final arrangement with `self.growth`. `apply` records `self.growth = growth` before acting, so `+` under a changed growth queues the depth that growth allows.

`queue_with_depth` starts from `headed = target_depth()` and queues one step per cell of depth change:

1. while above `wanted`, a step on the current target arrangement one depth lower (the summary gives a cell back);
2. each arrangement step, at `min(headed, wanted)`;
3. while below `wanted`, a step on the final arrangement one depth higher (the summary takes a cell).

`queue_steps` spreads the scan's `TILE_ANIMATION_MILLIS` over all of them. `advance` sets `self.depth = step.depth` and holds `settled_held()`. `settled_held` re-divides every cell back to its ask when the depth differs from the held one, as it already does when the cell count changes or a cell asks for more than it holds.

`settled_held` takes `summary_depth` from `self.depth` and `summary_span` from `wanted_span`. `drawn_held` does the same when it works the layout out fresh (no sync yet, or the cell count changed under `self.held`); otherwise it returns `self.held` as it stands. `resize_for_focus` queues its step at `self.depth`. `admitted` and `add` check `fits` on the logical cell count, so depth never refuses a cell.

### Layout

`column_layout` returns the deepest `d <= held_depth` with `d <= columns(count + d - 1, growth)[0]` (`d = 1` always qualifies). It also returns the column heights over exactly `count + d - 1` positions. Its callers:

- `Grid::new` divides column 0 into `height - d + 1` pieces; the summary is one pane over `d` positions. Every later column starts at cell `opens_at - d + 1`. Logical cell `n >= 2` sits at position `n + d - 1`.
- `content_widths` builds one width per position, drains positions `2..=d`, and swaps in the summary's width from `Grid::new`. The app therefore measures each cell where it draws.
- `wanted_span` counts its columns as `column_layout(self.count(), self.depth, self.growth).widths.len()`.

`Grid::column_of` maps cell `n` to the column of position `n + d - 1` (the summary maps to position 1). `Grid::lanes` lists cell 1 for every position up to `d`.

Pushed cells use the travel the grid already had. `moving_cell` compares `column_of` before and after, so a cell pushed over a column boundary wraps the way it does when a cell closes.

### Focus and clicks

Focus is logical throughout (`focused_cell`, `focus_at`, `focus_cell`, `cycle_focus`).

`focus_step` walks `Grid::lanes`. Column 0's lane lists cell 1 once per summary position, and a landing on the current cell is skipped. So:

- Down from the summary reaches the first cell under it.
- Up from that cell reaches the summary.
- Left from any cell beside the summary lands on it.

`cell_at` hit-tests the resolved panes. The summary is one pane over all its positions, so a click anywhere on it returns cell 1, which `host.rs` hands to `focus_cell`.

### With `widen_summary`

`Grid::new` divides column 0 first, so when the summary's row comes up its rect already has its depth-`k` height. `reach(opened, widths, summary, wanted, settings)` then widens it over the next columns up to the held `summary_span`. It stops at the first column whose cells would not fit below the summary at `shared_run(cells, min_tile_height)`.

A deeper summary widens over the columns its depth opens. A summary that fills column 0 cannot widen. `Turns::of` compares the spans of the two grids, so a step that changes both depth and span plays the widening or narrowing turns.

Defaults: cargo-tile `DEFAULT_WIDEN_SUMMARY = false`, cargo-handler `true`.

### Why the loop settles

Depth sets the span, the span sets the summary's width, the width sets how many rows it wraps to, and those rows set the depth. Each link moves one way:

- A deeper summary is never wider. More positions open the same number of columns or more, each narrower, and a lower bottom edge lets `reach` cover the same columns or fewer.
- A narrower summary never wraps to fewer rows.
- More rows never ask for less depth.

Giving back runs the chain in reverse. Depth is bounded by column 0 and `fits`, so the chain stops. A second `sync` with unchanged demands queues nothing and leaves the motion `Settled`.

## Invariants

- The summary never covers a cell. It takes positions: the layout has `count + k - 1` of them and every logical cell has its own.
- At `k = 1` the arrangement is the grid without depth (`column_layout(count, 1, ..)` is `columns(count, ..)`). In column 0 the summary's shortfall is served before the focus ring at every depth, `k = 1` included.
- Cell numbers stay logical: the summary is 1 and the cells after it are 2, 3, ..., whatever `k` is. `TileContent::Empty(n)`, `focus_cell`, `cell_at`, `cycle_focus`, `cells` and `cell_of` all use logical numbers. Only `Grid` and `column_layout` know positions.
- Depth never costs a running command its cell. Admission and `+` check `fits` on the logical count. Depth is capped by column 0's height and by `fits` on positions. At the cap the summary is clipped.
- Focus never changes `k`. The depth rule passes `ColumnFocus::Outside`, `shares` serves the summary before the ring, and `resize_for_focus` keeps `self.depth`.
- No setting: depth is always on in both apps, and `widen_summary` keeps its own meaning.
- No public API: every depth item is private to `grid.rs`.
- The apps' drawing knows nothing of depth. The summary draws into whatever rect it gets, and each app measures at the widths `content_widths` hands it.
- Every reader of the laid-out columns resolves depth through `column_layout`: `Grid::new`, `content_widths` and `wanted_span`.
- Span and depth describe the same grid: the held `summary_span` comes from `wanted_span`, which counts `column_layout`'s columns.
- One queued step per cell of depth change: give-backs come before the arrangement steps, takes come after, and the arrangement steps run at `min(headed, wanted)`.
- A steady summary queues nothing: the bare re-divide is queued only when `settled_held() != self.held`.

## Calibration and gotchas

- `TILE_DEMAND_STEP` = 3 and `TILE_BORDER_ROWS` = 2 (`crates/tui_pane/src/tiles/constants.rs`). `demanded_rows` rounds content up to a multiple of 3 and adds 2, so depth moves on whole demand steps, not on each wrapped row.
- `MIN_TILE_HEIGHT` = 3 is three floors: on every ask in `shares`, on each position in `fits`, and on each cell `reach` checks below a widened summary.
- `TILE_ANIMATION_MILLIS` = 720 is one scan's change, depth steps included, spread over its steps. `MIN_STEP_MILLIS` = 60 is the per-step floor, and past `MAX_PENDING_STEPS` = 64 the rest settle in one move. A frame counts as settled only after 720 ms without change.
- `initial_rows` defaults to 4. Depth adds positions, so deepening can cross `initial_rows` and rearrange the whole grid. Four cells under `redistribute(4)` lay out as `[4]` at `k = 1`, `[3, 2]` at `k = 2` and `[3, 3]` at `k = 3`.
- A step can carry a depth its column 0 no longer holds, when the growth changed between steps. Every reader therefore resolves depth through `column_layout` instead of reading `Step::depth` or `HeldCellLayout::summary_depth` as given.
- `HeldCellLayout::summary_span` is the span the summary asks for, but `reach` in `Grid::new` clamps the drawn span, so the two can differ. At 80 by 40 with four cells under `widening(4)`, depth 3 holds span 2 and draws span 1.
- On a live machine depth follows the load, so the window size that gives two or three cells shifts with what is running.
- A column can show a blank bottom row while a cell is mid-move. Only a settled frame shows a real gap.
- The workspace lint denies `unreachable!`, so `column_layout`'s loop returns at depth 1 instead.
- The tests live in `grid.rs`'s `mod tests::summary_depth`, not under `tests/`, because they need private items. They are pure and run in milliseconds. Helpers: `held_at_depth` and `deep_grid`.
  - Depth rule: `the_summary_counts_as_the_fewest_cells_that_hold_it`, `the_summary_gives_a_cell_back_once_it_fits_in_fewer`, `the_summary_never_takes_the_grid_past_fits`.
  - Layout: `a_deeper_summary_pushes_every_cell_one_place_on`, `a_deeper_summary_tiles_its_column`, `a_capped_summary_places_every_cell_without_an_empty_column`, `widths_follow_capped_summary_positions_before_the_next_sync`.
  - Division and focus: `the_summary_is_served_before_the_focus_ring`, `moving_focus_never_changes_the_summary_depth`, `the_arrows_move_by_position_beside_a_deep_summary`, `a_click_on_any_part_of_a_deep_summary_focuses_it`.
  - Queuing and motion: `adding_under_new_growth_queues_its_summary_depth`, `cell_numbers_hold_while_the_summary_deepens`, `deepening_travels_the_way_a_closing_cell_does_in_reverse`, `a_steady_summary_queues_nothing` (widen off and on).
  - Widening: `a_deeper_summary_widens_over_the_columns_its_depth_opens`. It covers depth 2 at span 2, depth 3 where `reach` stops at one column, and a capped depth under changed growth where `wanted_span` counts the resolved columns.

## Why

- **One resolver.** `Grid::new`, `content_widths` and `wanted_span` must agree on which positions exist. Clamping the depth separately in `Grid::new` and `content_widths` left phantom positions, and widths measured at positions the cells did not draw at. The app then wrapped text to the wrong width and asked for the wrong rows. `column_layout` returns the depth and the columns together, over exactly `count + d - 1` positions.
- **Logical numbering.** The public API addresses cells by number: `focus_cell`, `cell_at` (clicks go through `host.rs`), the number an empty cell carries in `TileContent::Empty(n)`, and Tab order. Renumbering on each depth change would make those numbers shift with the load. Only geometry needs positions, so only `Grid` knows them.
- **Clip instead of closing a cell.** A running command's cell is the display itself. Depth spends only the positions `fits` leaves once every cell is placed. Past that the summary is clipped, as it was before depth existed, and no cell is closed or refused for it.
- **Rebalance before deepening.** The depth rule measures the share after idle cells lend their spare rows, so a cell is pushed on only when column 0 has nothing left to give. Asks of `[30, 3, 3, 3]` stay at depth 1.
- **Served before the focus ring.** Depth comes from the summary's share of column 0. If the ring were served first, an arrow key onto a busy cell in column 0 would shrink that share and push cells around. Serving the summary first makes `k` depend on the asks alone.
- **Fresh asks every frame.** Depth is read from the asks the app just measured, not from held rows, so the summary gives a cell back on the frame its ask fits in fewer.
- **Shared with cargo-handler.** The grid is one implementation in `tui_pane`, and both apps' summaries can outgrow their cell. Limiting depth to cargo-tile would take a `TileGrowth` field, and depth has nothing to tune.
