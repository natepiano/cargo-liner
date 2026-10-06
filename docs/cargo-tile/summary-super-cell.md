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

### Phase 3 — Summary depth with `widen_summary`, changelogs and the visual check  · status: done

#### As-built

- `wanted_span` counts its columns as `column_layout(self.count(), self.depth, self.growth).widths.len()`, so the requested span is worked out over the columns `Grid::new` lays out, even when a step carries a depth its column 0 no longer holds. `column_layout` is the one depth resolver for `Grid::new`, `content_widths` and `wanted_span`; `settled_held` and `drawn_held` take `summary_span` from `wanted_span`, so span and depth describe the same grid.
- With `widen_summary` on, column 0 is divided first, so `reach` sees the taller summary rect: a deeper summary widens over the columns its depth opens, and `reach` stops it at a column whose cells no longer fit below it. A step that changes both depth and span plays the existing `Turns`.
- The loop depth → span → width → wrapped rows → depth settles and cannot oscillate, because every link is monotone; a second `sync` with the same demands queues nothing and motion is `Settled`.
- `## [Unreleased]` entries: `tui_pane` `### Changed` (the summary counts as two cells, then three, up to what its column holds, and gives each back once it fits in fewer; cell numbers stay logical, the summary is served before the focus ring, and it is clipped at the fit limit as before); `cargo-tile` `### Changed` (the same for the command cells, and with `widen_summary` on it widens over the columns its depth opens; cargo-tile's default is off); `cargo-handler` `### Added` (the same for the agents summary, with `widen_summary` on by default).
- Checked on screen: screenshots at two and three cells and recordings of the summary taking a cell and giving it back, judged on settled frames only; no rendering defects.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `wanted_span` counts its columns through `column_layout`
- `crates/tui_pane/src/tiles/grid/tests/summary_depth.rs` — 16 pure tests; `a_steady_summary_queues_nothing` covers widen off and widen on; `a_deeper_summary_widens_over_the_columns_its_depth_opens` covers depth 2 at span 2, depth 3 where `reach` stops at one column, and a capped depth under changed growth where `wanted_span` uses the resolved columns
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md` — the Unreleased entries

**Gotchas:**
- `HeldCellLayout::summary_span` is the span the summary asks for; `reach` in `Grid::new` clamps the drawn span, so the two can differ (depth 3 under `widening(4)`: held 2, drawn 1).
- On a live machine the summary's depth follows the load, so the window size that gives two or three cells shifts with what is running.
- A frame counts as settled only after `TILE_ANIMATION_MILLIS` (720 ms) without change; judge gaps on screen only then.
