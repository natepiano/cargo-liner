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

### Phase 2 — The summary counts as `k` cells  · status: done

#### As-built

- A summary that does not fit its cell after the normal rebalancing counts as the fewest cells of column 0 that hold it (its **summary depth** `k`), up to what that column holds and never past `fits`, pushing every other cell one position on through the existing travel; it gives each cell back once `k - 1` cells hold it. At the cap it is clipped as before. `k = 1` is the old grid. Cell numbers stay logical (summary 1, then 2, 3, …); cell `n` sits at position `n + k - 1`.
- State: `HeldCellLayout::summary_depth` (positions the summary covers, its own included; `rows` stays one entry per logical cell), `Step::depth` (the depth a step leaves), `TileGrid::depth` (the depth `slots` is laid out at) and `target_depth()` (the last pending step's depth, else `self.depth`).
- Division: free `fits(area, count, growth, settings)`; `enum ColumnHead { Summary(usize), Cell }` and `shares(wants, height, head, focused, floor)` — under `Summary(k)` the even split weights the summary `k` and its shortfall is served before the focus ring, so focus never changes `k`; `summary_share(wants, height, depth, growth, floor)`; `summary_depth(area, wants, growth, settings)`, the depth rule, read every frame from the fresh asks over the full `area.height`.
- Queuing: `queue(steps)` works out the wanted depth and calls `queue_with_depth(steps, wanted)`: give-back steps first, arrangement steps at `min(headed, wanted)`, added-cell steps after, one step per cell. `sync` computes the wanted depth once and calls `queue_with_depth` directly. `apply` records `self.growth = growth` before acting. Admission and `+` check `fits` on the logical count.
- Layout: `column_layout(count, held_depth, growth) -> ColumnLayout { widths, summary_depth }` resolves the laid-out depth — the deepest `d <= held_depth` with `d <= columns(count + d - 1)[0]`, over exactly `count + d - 1` positions — for both `Grid::new` and `content_widths`. `Grid` stores its depth; `column_of` and `lanes` map logical cells to positions; `focus_step` Up/Down skip the summary's own rows; a click anywhere on the summary focuses it. `wanted_span` counts columns at the unclamped `self.depth`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — summary depth through state, division, queuing, layout and focus; opens with `#![allow(clippy::self_named_module_files, reason = ...)]` for the `grid/tests/` layout and mounts `mod summary_depth;` inside `mod tests`
- `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs` — 15 pure tests, among them `a_steady_summary_queues_nothing` (widen off), `deep_grid()` and `held_at_depth`

**Binds later work:** "Summary depth with `widen_summary`, changelogs and the visual check" builds on `column_layout` as the one depth resolver, `wanted_span` at `self.depth`, `queue_with_depth`, `sync` queuing a bare re-divide only when `settled_held() != self.held`, and the tests in `summary_depth.rs`.

**Gotchas:** A step can carry a depth its column 0 no longer holds (the growth changed between steps), so every reader of the laid-out columns resolves the depth through `column_layout`. The workspace lint rejects `unreachable!`. A column can show a blank bottom row while a cell is mid-move; only a settled frame shows a real gap.

**Ruled out:** Clamping the depth separately in `Grid::new` and `content_widths` — it left phantom positions and widths measured at the wrong positions.

### Phase 3 — Summary depth with `widen_summary`, changelogs and the visual check  · status: todo

#### Work Order

**Goal:** with `widen_summary` on, a deeper summary widens over the columns its depth lays out and settles without requeuing; the change is in all three changelogs and is seen on screen at two and three cells.

**Spec:**
- With `widen_summary` on, column 0 is divided first, so `reach` sees the taller summary rect. It stops widening at any column whose cells no longer fit below the summary. The loop runs depth → span → width → wrapped rows → depth. Every link is monotone: more depth means less span and narrower columns, and narrower means more rows. So the loop settles in at most `N` steps and cannot oscillate. A test asserts that a second sync with the same demands queues nothing.
- `wanted_span` (`grid.rs:434`) already counts columns at the grid's depth, `columns(self.count() + self.depth - 1, self.growth).len()`, but with `self.depth` unclamped. It counts `column_layout(self.count(), self.depth, self.growth).widths.len()` instead (`column_layout`, `grid.rs:1233`), so a step whose depth column 0 no longer holds asks for a span over the same columns `Grid::new` lays out. `settled_held` (`:406`) and `drawn_held` (`:386`) take their `summary_span` from it, so span and depth always describe the same grid.
- A step that changes both depth and span plays the existing `Turns` (`grid.rs:1753`): `Turns::of` (`:1776`) already compares the spans at either end, and `Grid::column_of` (`:1379`) already maps cells to positions.
- Tests, in `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs`:
  - `a_steady_summary_queues_nothing` gains the widen-on case: under `widening(4)`, a summary that settles deeper and wider queues nothing on a second `sync` with the same demands.
  - `a_deeper_summary_widens_over_the_columns_its_depth_opens`: under `widening(4)` with `summary_width` wider than one column, the summary's settled `summary_span` is worked out over the columns of the deeper grid, and `reach` stops it at a column whose cells no longer fit below the taller summary.
  - The existing widen tests (`grid.rs:2254`–`2355`) stay green.
- CHANGELOG entries, one line each, under `## [Unreleased]`:
  - `crates/tui_pane/CHANGELOG.md`, `### Changed`: `TileGrid` lets the summary count as two cells, then three, up to what its column holds, whenever the normal rebalancing leaves it short, pushing every other cell one place on; it gives each cell back once it fits in fewer. Cell numbers stay logical, the summary is served before the focus ring in its column, and at the fit limit it is clipped as before.
  - `crates/cargo-tile/CHANGELOG.md`, `### Changed`: a summary too long for its cell takes the next cells of its column, pushing the command cells on, and gives them back once it fits; at the fit limit it is clipped as before.
  - `crates/cargo-handler/CHANGELOG.md`, `### Added` (its Unreleased section holds only `Added`): the same rule for the agents summary, with `widen_summary` on by default.
- Visual check before merge (unit director): run this worktree's cargo-tile in a terminal short enough, with enough cargo commands running, that the summary counts as two cells, then three; take a screenshot at each. Record a cell being added and given back with `/screen_record` (1 to 60 s). Judge a screenshot only once the grid has settled (no change for longer than `TILE_ANIMATION_MILLIS`, 720 ms): a column can show a blank bottom row while a cell is mid-move, and that is not a gap.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `wanted_span` counts its columns through `column_layout`, at the depth `Grid::new` lays out
- `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs` — the widen-on steady test and the widening test
- `crates/tui_pane/CHANGELOG.md` — the `Changed` entry
- `crates/cargo-tile/CHANGELOG.md` — the `Changed` entry
- `crates/cargo-handler/CHANGELOG.md` — the `Added` entry

**Seats:** 1 writer + 1 tester — the code change and the changelogs split from the tests by file; the two sets were independent in the summary-depth phase, and the test module is already mounted, so the tester needs no hub line.
- `impl` — `crates/tui_pane/src/tiles/grid.rs` and the three `CHANGELOG.md` files; hub: `crates/tui_pane/src/tiles/grid.rs` (the shared test helpers in its `mod tests`)
- `test` — `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs`: the widen-on case of `a_steady_summary_queues_nothing` and `a_deeper_summary_widens_over_the_columns_its_depth_opens`, from the Spec alone.

**Constraints from prior phases:** Summary depth is built in `crates/tui_pane/src/tiles/grid.rs`; every item below is private to it.
- State: `HeldCellLayout::summary_depth`, `Step::depth`, `TileGrid::depth` (the depth `slots` is laid out at, starts at 1) and `target_depth()` (`:752`, the last pending step's depth, else `self.depth`). `advance` moves `step.depth` into `self.depth`.
- Division: the free `fits(area, count, growth, settings)` (`:1629`); `enum ColumnHead { Summary(usize), Cell }` (`:1525`) and `shares(wants, height, head, focused, floor)` (`:1555`), where under `Summary(k)` the summary has weight `k` and its shortfall is served before the focus ring; `summary_share(wants, height, depth, growth, floor)` (`:1642`); `summary_depth(area, wants, growth, settings)` (`:1662`, the depth rule). Both divide column 0 over the full `area.height`, so the span reaches the depth only through the summary's measured rows.
- One depth resolver: `column_layout(count, held_depth, growth) -> ColumnLayout { widths, summary_depth }` (`:1233`) picks the deepest `d <= held_depth` with `d <= columns(count + d - 1)[0]` and lays out exactly `count + d - 1` positions. A step can carry a depth its column 0 no longer holds (the growth changed between steps), so every reader of the laid-out columns calls it: `Grid::new` (`:1255`) and `content_widths` (`:454`) do, and `wanted_span` is the one reader left on the raw `self.depth`.
- `Grid` stores its depth; `Grid::column_of` maps cell `n` to position `n + depth - 1` (the summary to position 1); `lanes` (`:1345`) lists the summary at each of its positions; `focus_step` Up/Down skip the summary's own rows.
- Queuing: `queue(steps)` (`:765`) works out the wanted depth from the final slots with `self.growth` and calls `queue_with_depth(steps, wanted)` (`:773`): give-back steps first, arrangement steps at `min(headed, wanted)`, added-cell steps after. `sync` (`:545`) computes its wanted depth once and calls `queue_with_depth` directly; with no cell moving and the depth where it is headed, it queues a bare re-divide step only when the grid is settled and `settled_held() != self.held`. `HeldCellLayout` equality covers `rows`, `focused`, `summary_span` and `summary_depth`, so the widen-on steady case holds exactly when a second sync's `settled_held()` matches on all four. `settled_held` re-divides from the fresh wants when `self.depth != self.held.summary_depth`.
- `apply` (`:1123`) records `self.growth = growth` before acting, so an action and the depth it queues use the same growth. Admission and `+` check `fits` on the logical count.
- grid.rs opens with `#![allow(clippy::self_named_module_files, reason = ...)]` (`:48`), because the private tests live in `grid/tests/` beside it; keep it. The workspace lint rejects `unreachable!`.
- Tests: `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs` is mounted as `mod summary_depth;` inside `grid.rs`'s `mod tests` (`:2108`) and opens with `use super::*;`, reaching the `mod tests` helpers (`seeded_grid`, `busy`, `redistribute`, `widening` at `:2235`, `wide_summary` at `:2244`, `blank_tally`, `paint`), `TileGrid::settle_for_test` (`:906`) and its own `deep_grid()` (three commands asking 7 rows, summary asking 12, `redistribute(4)`, settled at depth 2) and `held_at_depth(rows, depth)`. It holds 15 tests; `a_steady_summary_queues_nothing` re-syncs `deep_grid()`'s demands and asserts `grid.pending.is_empty()` and `GridMotion::Settled` with `widen_summary` off.
- Baseline: `test tui_pane` 918/918, `test cargo-tile` 725/725, `test cargo-handler` 148/148, with no cargo-tile or cargo-handler code or expectation changed by depth.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` green
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` green, including both tests named in the Spec and the existing widen tests
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` green
- `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` green
- Screenshots of cargo-tile with the summary at two cells and at three cells, and a recording of a cell being added and given back, checked before merge.
