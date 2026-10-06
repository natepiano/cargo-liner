# Summary super cell

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** When the summary does not fit its cell, it counts as two cells, then three, up to what column 0 holds, pushing every other cell on, and gives each cell back once it fits in fewer. Also fixes the reader test that fails in any checkout whose path contains "cleanup".

> **Production: cargo-port-cleanup** — unit `cargo-tile`; production doc `docs/cargo-port/remove-running-section-production.md`

## Delegation Context

- **Project:** the cargo-liner workspace. `tui_pane` is the TUI framework and owns the tile grid; `cargo-tile` watches the cargo runs on this machine and only measures and draws what goes in each cell; `cargo-handler` watches Claude Code and Codex agents on the same grid.
- **Project started:** 2026-10-06T21:16:36.370+00:00
- **Stack:** Rust 2024 edition, ratatui 0.30.2, crossterm 0.29; cargo nextest (through `verify.sh`). The reader scenarios are a Python 3 script driven by a Rust unit test.
- **Layout:**
  - `crates/tui_pane/src/tiles/` — the grid: `grid.rs` (arrangement, division, depth, motion, focus, and its `#[cfg(test)] mod tests`), `draw.rs`, `constants.rs`, `settings.rs`, `growth.rs`, `host.rs`
  - `crates/cargo-tile/src/` — `render.rs` (the summary's demand and drawing), `settings.rs` (the Settings overlay rows), `shim_registration/` (reader scenarios)
  - `crates/cargo-handler/src/` — `render.rs`, `constants.rs`
  - `crates/{tui_pane,cargo-tile,cargo-handler}/CHANGELOG.md`
- **Key files:**
  - `crates/tui_pane/src/tiles/grid.rs` — the whole grid. `TileDemands` (`:98`), `HeldCellLayout` (`:269`: rows per cell, summary first, plus `focused` and `summary_span`), `Transition` (`:281`), `Step` (`:297`), `TileGrid` (`:311`), `drawn_held` (`:372`), `settled_held` (`:391`, re-divides only when some cell asks for more than it holds, or a cell opens or closes), `wanted_span` (`:416`), `count()` (`:428`, the logical cell count), `content_widths` (`:437`), `sync` (`:522`, every frame), `admitted` (`:618`), `close` (`:657`), `add` (`:668`), `remove` (`:690`), `target` (`:715`), `queue` (`:732`), `advance` (`:764`), `resize_for_focus` (`:789`), `settle_for_test` (`:839`), `focus_step` (`:857`), `cycle_focus` (`:900`), `focused_cell` (`:917`), `focus_at` (`:928`), `focus_cell` (`:940`), `cell_at` (`:951`), `fits` (`:962`), `placements` (`:990`), `cells`/`cell_of`/`content_of` (`:1077`–`1096`), `Grid` (`:1150`), `Grid::new` (`:1172`, divides column 0 first), `Grid::lanes` (`:1243`), `Grid::column_of` (`:1271`), `summary_span` (`:1285`), `reach` (`:1304`), `below` (`:1336`), `columns` (`:1368`, every column's cell count from the total count alone: one column up to `initial_rows`, default 4, then the smallest square), `shares` (`:1435`, even split at `:1440`, focus ring served first at `:1461`, no cell below `MIN_TILE_HEIGHT`), `apportion` (`:1495`), `cell_wants` (`:1530`, rounds every ask up to a whole `TILE_DEMAND_STEP` plus two border rows), `shared_run` (`:1547`), `Turns` (`:1564`), `moving_cell` (`:1658`), `wrapping_cell` (`:1712`), `edge_rect` (`:1795`), `closing_rect` (`:1829`), `position` (`:1858`), `lerp_rect` (`:1870`). Test module from `:1913`: helpers `test_area` (80×40), `demanded_rows`, `even`, `quiet`, `blank_tally`, `paint`, `add_new`, `redistribute`, `widening`, `wide_summary`, `busy`, `column_rows` (`:2426`, calls `shares`), `drawn` (`:2781`), `seeded_grid` (`:2916`), `shown` (`:2923`); widen tests `:2061`–`2163`, `shares` tests `:2439`–`2578`, focus tests `:3365`–`3497`
  - `crates/tui_pane/src/tiles/constants.rs` — `TABLE_CELL` = 1 (`:8`), `MIN_TILE_HEIGHT` = 3 (`:18`), `TILE_BORDER_ROWS` = 2, `TILE_DEMAND_STEP` = 3 (`:31`), `PROGRESS_SCALE`, `TILE_ANIMATION_MILLIS` = 720, `MAX_PENDING_STEPS` = 64
  - `crates/tui_pane/src/tiles/settings.rs` — `TileSettings`; `demanded_rows` rounds content up to a whole demand step, plus the two border rows
  - `crates/tui_pane/src/tiles/growth.rs` — `TileGrowth { initial_rows, fill, widen_summary }` (`widen_summary` at `:35`; it widens the summary across the top of later columns, never taller)
  - `crates/tui_pane/src/tiles/draw.rs` — `draw_tile_grid` (`:119`: `set_layout`, `content_widths`, the app's demands, `add_readout_rows`, `sync`, `placements`), `add_readout_rows` (`:234`, one readout row per cell), `draw_rows_readout` (`:290`, writes the content-row count red when the contents do not fit)
  - `crates/tui_pane/src/pane/frame.rs` — `share_borders` (`:153`): a rect grows one line onto the border it shares with the neighbour below and to the right
  - `crates/cargo-tile/src/render.rs` — `tile_demands` (`:310`) measures the summary at the width it will be drawn at, over `summary_rows` (`:610`), which `draw_summary` (`:568`) lays out; `draw_process_table` stops at `remaining.height == 0` (`:1432`); `draw_path_group` truncates the last group's table (`:1558`)
  - `crates/cargo-tile/src/interaction.rs` — `focus_cell(TABLE_CELL + 1)` at `:163` means the first cell after the summary
  - `crates/cargo-tile/src/census/scan.rs` — `focus_cell(TABLE_CELL + 1)` in a test at `:3940`
  - `crates/cargo-tile/src/constants.rs` — `DEFAULT_WIDEN_SUMMARY = false` (`:113`)
  - `crates/cargo-handler/src/constants.rs` — `DEFAULT_WIDEN_SUMMARY = true` (`:78`)
  - `crates/cargo-tile/src/settings.rs` — the Settings overlay's `Capture` section (`:162`) and `Commands` section (`:166`); `tui_pane` renders each section heading as `<name>:` (`crates/tui_pane/src/overlays/settings.rs:731`), inside a centred, titled, bordered popup (`crates/tui_pane/src/app_settings/draw.rs:33`)
  - `crates/cargo-tile/src/shim_registration/reader_scenario.py` — `settings_screen` (`:697`) returns the whole terminal; the `root-headings` assertions (`:1030`–`1050`), the failing one at `:1046`
  - `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — `reader_ignores_foreign_owned_account_and_reports_its_owner_in_settings` (`:287`) runs scenario `root-headings`
- **Test lanes:** `tui_pane` — `crates/tui_pane/tests`; `cargo-tile` — `crates/cargo-tile/tests`; `cargo-handler` — none. The grid's tests need private items, so they live in `grid.rs`'s test module tree, not under `tests/`.
- **Build:**
  - `bash ~/.claude/scripts/delegate/verify.sh check tui_pane`
  - `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
  - `bash ~/.claude/scripts/delegate/verify.sh check cargo-handler`
- **Test:**
  - `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`
  - `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
  - `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`
- **Lint:**
  - `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane`
  - `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
  - `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Style:** `run-end /clippy style-only auto-proceed`
- **Invariants:**
  - All grid logic changes are in `crates/tui_pane/src/tiles/grid.rs` (design scope, user-approved). cargo-tile's drawing needs no change, since the summary draws into whatever rect it gets.
  - The summary never covers a cell (user). It counts as `k` cells and pushes every other cell on; at `k = 1` the grid is exactly today's.
  - No setting: summary depth is always on, and `widen_summary` stays as it is (user-approved call).
  - cargo-handler gets the same rule, since the grid is shared (user-approved call). Making it cargo-tile-only would take a `TileGrowth` field.
  - No public API change: every new item is private to `grid.rs` (style rule `public-api-changes-require-explicit-approval`).
  - Cell numbers stay logical: the summary is 1 and the cells after it are 2, 3 and so on, whatever `k` is. Only geometry uses positions (user-approved call).
  - At the fit limit the summary is clipped, not given a running command's cell. The grid never closes a running command's cell to make room; that would be new behaviour the request did not ask for (user-approved call).
  - Tests run through `verify.sh` (cargo nextest, user rule). Format with `cargo +nightly fmt` only, never stable `cargo fmt` (user rule). No test may take seconds: the new grid tests are pure and run in milliseconds (user rule).

## Phases

### Phase 1 — Reader test reads the Capture section, not the whole screen  · status: done

#### As-built

The `root-headings` reader scenario checks for the removed capture-status cleanup wording only inside the Settings overlay's Capture section. `capture_settings_rows(rendered, account)` crops the popup to its interior columns, between its left and right border glyphs on the `Capture:` row, from that row through the row before `Commands:`. It asserts that the account's `yours · 1 capture ·` row is inside the crop, so a mis-cut fails loudly. A checkout or target path containing "cleanup" in the tile grid around the overlay no longer fails the test. Cleanup wording back in the Capture section still does.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario.py` — `capture_settings_rows` and the narrowed assertion in `root-headings`

**Gotchas:** `assert 'configured' not in settings.lower()` in the same scenario still reads the whole screen, so a path containing "configured" would fail it.

### Phase 2 — The summary counts as `k` cells  · status: todo

#### Work Order

**Goal:** a summary that does not fit its cell after the normal rebalancing counts as the fewest cells of column 0 that hold it, up to what that column holds, pushing every other cell on through the grid's existing travel, and gives each cell back once it fits in fewer.

**Spec:**

*The model.*
- The grid gains a **summary depth** `k`, the number of cells the summary counts as. The arrangement holds the summary at positions `1..=k` and every other cell after it, so the grid lays out `k - 1` more cells than it does today. `columns`, `shares`, `fits`, the motion and the focus ring all see a grid of `k + (number of other cells)` cells and behave exactly as they do today. The only difference is that the summary's `k` positions at the top of column 0 are drawn as one cell.
- Inside column 0 the summary is one piece of `shares` whose even starting split is `k` units (a weight of `k` in `apportion`, `grid.rs:1495`). The cells below it get one unit each, and the normal rebalancing follows.

*The depth rule (verbatim from the approved design).*

```
k = the smallest k, starting at 1, for which the summary's rows after the
    normal rebalancing of a grid with the summary counted as k cells
    hold what the summary asks for
k may not exceed the number of cells column 0 holds in that grid
k may not make the grid too big to fit (fits, grid.rs:962)
when no k holds it, k is the largest allowed and the summary is clipped as today
```

`k = 1` is today's grid. The summary gives a cell back as soon as `k - 1` cells hold it. "The summary's rows" is its allocation from `shares` (the `PaneAxisSize::Fixed` value), compared with its entry in `cell_wants`; that allocation does not depend on focus, because the summary is served first in column 0 (below).

Example: a 40-row terminal, `initial_rows` 4, the summary plus commands A, B and C. Each command asks for 10 rows.

| summary asks | `k` | grid laid out as | summary gets |
|---|---|---|---|
| 9 | 1 | one column: S, A, B, C | 10 |
| 14 | 2 | column 0: S, S, A; column 1: B, C | 27 |
| 30 | 2 | same | 30 (A gives up its 3 spare rows) |
| 35 | 3 | column 0: S, S, S; column 1: A, B, C | 40 |
| 45 | 3, the column is full | same | 40, clipped as today |

Counting as more cells can change the grid's shape. In row 2 the summary becomes two cells, the grid holds 5 cells, and 5 is past `initial_rows`, so the single column becomes two columns. This is the existing rule for a 5-cell grid, applied unchanged.

*Private items (names set at plan compile; the tester writes against them).*
- `HeldCellLayout` gains `summary_depth: usize` — positions the summary counts as, its own included; 1 is today's grid. Held with the rows, like `summary_span`, so a change of depth is a change the grid travels through. `rows` stays one entry per logical cell, the summary's first, so a layout covers `rows.len() + summary_depth - 1` positions. Every construction sets it, including `TileGrid::new` (1), the test helper `even` (1) and the literal `HeldCellLayout` in the test at `grid.rs:2671`.
- `Step` gains `depth: usize`, the depth that step leaves. `TileGrid` gains `depth: usize` (starts at 1), the depth `slots` is laid out at; `advance` moves `step.depth` into it alongside `step.slots`. The depth the grid is headed for is the last pending step's `depth`, else `self.depth` (the counterpart of `target`).
- `fits` (`grid.rs:962`) moves its body to a free `fn fits(area: Rect, count: usize, growth: TileGrowth, settings: &TileSettings) -> bool`; `TileGrid::fits` calls it with `self.area` and `self.settings`.
- `enum ColumnHead { Summary(usize), Cell }` — what opens a column: the summary at that depth (column 0, any `k`), or an ordinary cell (every other column). `shares` becomes `shares(wants: &[u16], height: u16, head: ColumnHead, focused: ColumnFocus, floor: u16)`. Under `ColumnHead::Summary(k)` the even split is `apportion` over weights `[k, 1, 1, …]`, and the summary's shortfall is served first out of the spare room, then the focus ring (when it is not on the summary), then the rest of the short cells in proportion. Under `ColumnHead::Cell` it behaves exactly as today: even split, ring first, the rest in proportion. The existing `shares` tests and `column_rows` pass `ColumnHead::Cell`.
- `fn summary_share(wants: &[u16], height: u16, depth: usize, growth: TileGrowth, floor: u16) -> u16` — the rows column 0 hands the summary when it counts as `depth` cells. Column 0 holds `columns(wants.len() + depth - 1, growth)[0]` positions, so its pieces are the summary plus the next `that - depth` entries of `wants`, divided by `shares(.., height, ColumnHead::Summary(depth), ColumnFocus::Outside, floor)`; the answer is the first piece.
- `fn summary_depth(area: Rect, wants: &[u16], growth: TileGrowth, settings: &TileSettings) -> usize` — the depth rule above, pure. Walk `k` up from 1: return `k` once `summary_share(wants, area.height, k, ..) >= wants[0]`; before stepping to `k + 1`, return `k` unless `k + 1` is allowed. `k` is allowed when `k <= columns(wants.len() + k - 1, growth)[0]` and `fits(area, wants.len() + k - 1, growth, settings)`. `k = 1` is always allowed.

*Where `k` is decided, and when it changes.*
- `sync` (`grid.rs:522`) runs every frame. It works out the wanted `k` from the fresh asks (`cell_wants`), the last frame's area and the growth settings. A `k` that differs from where the grid is headed queues one step per cell added or given back, the same way a cell opening or closing is queued (`queue`, `grid.rs:732`). `Step` and the held snapshot carry `k`, so a transition knows the depth it is moving from.
- `k` is read from the fresh asks, not from the sticky held rows, so a cell goes back as soon as it is not needed. The 3-row demand step keeps one row of rewrap from flipping `k`.
- A resize changes `H`, so the next frame's `sync` may change `k`. That change travels as a step.
- Step order (set at plan compile): one change's steps carry their depth around the arrangement steps. The wanted `k'` is `summary_depth` over the last arrangement of the change (the target when the change moves no cell). Give-back steps (target slots, depth one less each) queue before the arrangement steps, the arrangement steps carry `min(headed, k')`, and added-cell steps (final slots, depth one more each) queue after them, so no step lays out more positions than the grid it ends at fits. `sync`, `add` and `remove` all queue through this. A change that moves no cell and leaves `k` where it is headed queues nothing, as today.
- A command arriving and `+` keep checking `fits` against the logical count, exactly as today (set at plan compile, from the fit-limit invariant): a cell that fits at `k = 1` opens, and the summary gives cells back to make room.
- `settled_held` carries `summary_depth: self.depth` and re-divides (rows from the fresh wants) when `self.depth` differs from `self.held.summary_depth`, as it does when a cell opens or closes. `drawn_held`'s fallback carries `self.depth`. `resize_for_focus` pushes its step at `self.depth`.

*Layout.*
- `Grid::new` lays out `held.rows.len() + held.summary_depth - 1` positions (`widths = columns(positions, growth)`). It never lays out a deeper summary than column 0 holds: the depth it uses is `held.summary_depth` reduced until `depth <= widths[0]` (a step can carry a depth its own arrangement no longer holds). Column 0's pieces are the summary plus the next `widths[0] - depth` cells, divided with `ColumnHead::Summary(depth)`; every other column's pieces are the cells at its positions, divided with `ColumnHead::Cell`. Cell `n` sits at position `n + k - 1`. `Grid::new` maps a number to a position; the resolved panes stay keyed by logical cell number, and the summary's pane covers all `k` positions. `held.focused` (a logical number) becomes the `ColumnFocus::Row` of that cell's piece within its column.
- `Grid` stores its depth. `Grid::column_of(n)` maps cell `n` to its position (the summary to position 1) before `position(&self.widths, ..)`, so `moving_cell`, `wrapping_cell`, `edge_rect`, `closing_rect` and `Turns::covers` see the right column.
- `content_widths` builds its per-position widths from `columns(self.count() + depth - 1, growth)`, with depth from `self.drawn_held().summary_depth`, and drops the summary's extra `depth - 1` positions before zipping the widths with `cells`.

*Focus ring.*
- The summary is served before the focus ring in column 0. Today the ring is served first. If it still were, moving focus onto a short cell in column 0 could take the summary's rows and add a cell to the summary, so an arrow key would reshape the grid. Serving the summary first keeps `k` independent of focus. Everywhere else the ring keeps its priority.
- Arrows move by position. Down from the summary lands on the cell under its last position. Left or Right from a cell beside any part of the summary lands on the summary. `lanes` (`grid.rs:1243`) lists the summary at each of its positions, and the step from it skips its own positions: in `focus_step`, Up and Down take the nearest row in that direction whose cell is not the focused one.
- Tab (`cycle_focus`, `grid.rs:900`) is unchanged: summary, then each cell in order.
- A click anywhere on the summary focuses it, because the summary's pane covers all `k` positions. `cell_at` (`grid.rs:951`) needs no change.

*Cell numbers.* Cell numbers stay logical: the summary is 1 and the cells after it are 2, 3 and so on, whatever `k` is. Only geometry uses positions, where cell `n` sits at position `n + k - 1`. An empty cell keeps showing the same number while `k` changes, and `focus_cell(TABLE_CELL + 1)` (`interaction.rs:163`, `census/scan.rs:3940`) still means the first cell after the summary. `cells`, `cell_of` and `content_of` (`grid.rs:1077`–`1096`) keep their numbers.

*Motion.* The existing motion handles it. Adding a cell to the summary moves every cell after it forward one position. Those are the same travels the grid already makes when a cell closes, played in reverse. A cell that crosses into the next column uses `wrapping_cell`, and a column that opens grows in from the right edge. The summary grows down into the room cell 2 left behind. Giving a cell back plays the reverse. Each change of `k` by one cell is one step, so a jump from 1 to 3 plays as two steps in order. `placements` needs no change beyond `Grid` reading the depth from `transition.held` and from the settled held.

*Narrow and short terminals.*
- **Short.** Units are smaller, so the summary needs more cells sooner. Every other cell keeps `MIN_TILE_HEIGHT`, because `k` never takes the grid past `fits`.
- **At the fit limit.** On a terminal so small that one more cell would not fit, the summary cannot add a cell and is clipped as today.
- **Narrow.** The summary wraps to more rows and so needs more cells. Nothing else changes.

*Consumers.* cargo-tile and cargo-handler change no code. If one of their tests fails because its summary now counts as more than one cell, check the new layout against the depth rule: when it follows the rule, update that test's expectation in its own file; when it does not, the defect is in `grid.rs` and is fixed there.

*Tests.* Unit tests (pure, millisecond-scale) in `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs`, mounted by `impl` as `mod summary_depth;` inside `grid.rs`'s existing `mod tests` and opening with `use super::*;`, which reaches every existing test helper and every `grid.rs` item:
1. `the_summary_counts_as_the_fewest_cells_that_hold_it`: the table above, through `summary_depth` and `summary_share` over `test_area()` (80×40) and `redistribute(4)` with wants `[asks, 10, 10, 10]`, including the existing rebalancing settling `k = 1` (idle commands giving the summary their spare) and the column cap.
2. `the_summary_gives_a_cell_back_once_it_fits_in_fewer`: a `TileGrid` synced to a tall summary settles deeper, and synced again to a short one settles at `k = 1`.
3. `a_deeper_summary_pushes_every_cell_one_place_on`: cell `n` sits at position `n + k - 1`, the last cell wraps to the next column, and a column opens when the count calls for one.
4. `a_deeper_summary_tiles_its_column`: the tally helper (`blank_tally`, `paint`) shows no gap and no double draw, and `summary.bottom() - 1` equals the next cell's top.
5. `the_summary_never_takes_the_grid_past_fits`: in an area where one more position does not fit, `summary_depth` stays put and the summary is clipped.
6. `the_summary_is_served_before_the_focus_ring`, and `moving_focus_never_changes_the_summary_depth`.
7. `the_arrows_move_by_position_beside_a_deep_summary` and `a_click_on_any_part_of_a_deep_summary_focuses_it`.
8. `cell_numbers_hold_while_the_summary_deepens`.
9. `deepening_travels_the_way_a_closing_cell_does_in_reverse`: mid-step placements (through `moving_cell` between a depth-1 and a depth-2 `Grid`, at several progress values) keep the summary's foot on the line where cell 2's top is.
10. `a_steady_summary_queues_nothing`, with `widen_summary` off: a second `sync` with the same demands queues nothing (Phase 3 adds the widen-on case).

The existing widen and focus tests stay green at `k = 1`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — depth through `HeldCellLayout`, `Step`, `TileGrid`, `sync`/`queue`/`advance`/`settled_held`/`drawn_held`, `fits`, `ColumnHead` and `shares`, `summary_share`, `summary_depth`, `Grid::new`/`column_of`/`lanes`, `focus_step`, `content_widths`; existing test call sites updated for the new field and the `shares` argument; the `mod summary_depth;` mount
- `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs` — new: tests 1–10 above
- `crates/cargo-tile/src/render.rs` — only a `#[cfg(test)]` expectation the depth rule moves, if any
- `crates/cargo-handler/src/render.rs` — only a `#[cfg(test)]` expectation the depth rule moves, if any

**Seats:** 1 writer + 1 tester — the depth threads through one file, `grid.rs`, so one writer holds it; the tests sit in their own child module file, written from the Spec's named private items.
- `impl` — `crates/tui_pane/src/tiles/grid.rs` and any consumer test expectation in `crates/cargo-tile/src/render.rs` or `crates/cargo-handler/src/render.rs`; hub: `crates/tui_pane/src/tiles/grid.rs` (the `mod summary_depth;` mount and the shared test helpers)
- `test` — `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs`: tests 1–10 from the Spec alone, against `summary_depth`, `summary_share`, `shares` with `ColumnHead`, `HeldCellLayout::summary_depth`, `TileGrid::depth`, `Grid::new`, `moving_cell` and the public grid calls.

**Constraints from prior phases:** Phase 1 changed only `reader_scenario.py`; nothing here depends on it.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` green
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` green, including tests 1–10 named in the Spec and every existing grid, widen and focus test
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` green
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` green
- No new test takes more than a few milliseconds.

### Phase 3 — Summary depth with `widen_summary`, changelogs and the visual check  · status: todo

#### Work Order

**Goal:** with `widen_summary` on, a deeper summary widens over the columns its depth lays out and settles without requeuing; the change is in all three changelogs and is seen on screen at two and three cells.

**Spec:**
- With `widen_summary` on, column 0 is divided first, so `reach` sees the taller summary rect. It stops widening at any column whose cells no longer fit below the summary. The loop runs depth → span → width → wrapped rows → depth. Every link is monotone: more depth means less span and narrower columns, and narrower means more rows. So the loop settles in at most `N` steps and cannot oscillate. A test asserts that a second sync with the same demands queues nothing.
- `wanted_span` (`grid.rs:416`) counts the columns of the grid as laid out at the grid's depth: `columns(self.count() + self.depth - 1, self.growth).len()`, not `columns(self.count(), ..)`. `settled_held` and `drawn_held` take their `summary_span` from it, so span and depth always describe the same grid.
- A step that changes both depth and span plays the existing `Turns` (`grid.rs:1564`): `Turns::of` already compares the spans at either end, and `Grid::column_of` already maps cells to positions.
- Tests, in `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs`:
  - `a_steady_summary_queues_nothing` gains the widen-on case: under `widening(4)`, a summary that settles deeper and wider queues nothing on a second `sync` with the same demands.
  - `a_deeper_summary_widens_over_the_columns_its_depth_opens`: under `widening(4)` with `summary_width` wider than one column, the summary's settled `summary_span` is worked out over the columns of the deeper grid, and `reach` stops it at a column whose cells no longer fit below the taller summary.
  - The existing widen tests (`grid.rs:2061`–`2163`) stay green.
- CHANGELOG entries, one line each, under `## [Unreleased]`:
  - `crates/tui_pane/CHANGELOG.md`, `### Changed`: `TileGrid` lets the summary count as two cells, then three, up to what its column holds, whenever the normal rebalancing leaves it short, pushing every other cell one place on; it gives each cell back once it fits in fewer. Cell numbers stay logical, the summary is served before the focus ring in its column, and at the fit limit it is clipped as before.
  - `crates/cargo-tile/CHANGELOG.md`, `### Changed`: a summary too long for its cell takes the next cells of its column, pushing the command cells on, and gives them back once it fits; at the fit limit it is clipped as before.
  - `crates/cargo-handler/CHANGELOG.md`, `### Added` (its Unreleased section holds only `Added`): the same rule for the agents summary, with `widen_summary` on by default.
- Visual check before merge (unit director): run this worktree's cargo-tile in a terminal short enough, with enough cargo commands running, that the summary counts as two cells, then three; take a screenshot at each. Record a cell being added and given back with `/screen_record` (1 to 60 s).

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `wanted_span` counts positions at the grid's depth
- `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs` — the widen-on steady test and the widening test
- `crates/tui_pane/CHANGELOG.md` — the `Changed` entry
- `crates/cargo-tile/CHANGELOG.md` — the `Changed` entry
- `crates/cargo-handler/CHANGELOG.md` — the `Added` entry

**Seats:** 1 writer + 1 tester — the code change and the changelogs split from the tests by file.
- `impl` — `crates/tui_pane/src/tiles/grid.rs` and the three `CHANGELOG.md` files; hub: `crates/tui_pane/src/tiles/grid.rs`
- `test` — `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs`: the widen-on case of `a_steady_summary_queues_nothing` and `a_deeper_summary_widens_over_the_columns_its_depth_opens`, from the Spec alone.

**Constraints from prior phases:** Phase 2 built summary depth in `crates/tui_pane/src/tiles/grid.rs`: `HeldCellLayout::summary_depth`, `Step::depth`, `TileGrid::depth` (the depth `slots` is laid out at), `fits(area, count, growth, settings)`, `ColumnHead { Summary(usize), Cell }` and `shares(wants, height, head, focused, floor)`, `summary_share`, `summary_depth(area, wants, growth, settings)`, the step order (give-backs before the arrangement steps, added cells after), `Grid::new` laying out `rows.len() + depth - 1` positions with the depth clamped to column 0, `Grid::column_of` mapping cell `n` to position `n + k - 1`, and `content_widths` over positions. The tests live in `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs`, mounted inside `grid.rs`'s `mod tests`; `a_steady_summary_queues_nothing` there covers `widen_summary` off. Admission and `+` still check `fits` on the logical count.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` green
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` green, including both tests named in the Spec and the existing widen tests
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` green
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` green
- Screenshots of cargo-tile with the summary at two cells and at three cells, and a recording of a cell being added and given back, checked before merge.
