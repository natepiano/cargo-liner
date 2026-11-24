# tile-count-row-and-notice

> **Status: IMPLEMENTATION PLAN — phased, direct-ready.** A full-width cell shows its "+N more" row, and a notice stays still when grid cells change height.

> **As-built disposition: amend** — `docs/cargo-tile/as-built/tile-fixes.md`

## Source

2026-11-22 11:37 EST (showrunner, from the change-row check of C375 and C374 on natedev, cargo-tile 0.2.78-dev from main a9dea4906):

1. A full-width cell with more processes than rows shows the "+N more" row (prepare_table_viewport, render.rs). Add a test for the full-width case.
2. The notice must not hide and slide in again when a grid cell changes height (your reslide trace). A click on [x] must not miss. Add a test.

## Delegation Context

- **Project:** cargo-tile — a terminal grid of the cargo processes on the machine; tui_pane — its shared TUI framework (tiles, toasts, input).
- **Project started:** 2026-11-22T16:41:37.664+00:00
- **Stack:** Rust 2024, ratatui, crossterm.
- **Layout:**
  - `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/render/` — cell tables, the hidden-process count row.
  - `crates/tui_pane/src/tiles/` — grid placement, motion and cell drawing.
  - `crates/tui_pane/src/toasts/` — notices: entrance, exit, layout, hit areas.
  - `crates/tui_pane/src/framework/` — input dispatch and hit tests.
  - `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — app-level notice behavior and its scenarios.
- **Key files:**
  - `crates/cargo-tile/src/render.rs` — `prepare_table_viewport` (about line 5895), `draw_hidden_process_count` (about line 5991), `draw_process_table` (about line 5693).
  - `crates/tui_pane/src/toasts/manager.rs`, `toast.rs`, `view.rs` — toast phases and card geometry.
  - `crates/tui_pane/src/toasts/render/` — toast drawing.
  - `crates/tui_pane/src/framework/hit_test.rs`, `framework/input.rs` — the [x] hit test.
- **Test lanes:** cargo-tile `crates/cargo-tile/tests/`; tui_pane `crates/tui_pane/tests/`.
- **Build:** `bash ~/.claude/scripts/direct/verify.sh check cargo-tile`
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test cargo-tile`, `bash ~/.claude/scripts/direct/verify.sh test tui_pane`
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`, `bash ~/.claude/scripts/direct/verify.sh lint tui_pane`
- **Invariants:** every test runs in under 1 s alone (user rule). No `#[allow]`. No duplicated code without approval.

## Phases

### Phase 1 — A full-width cell shows "+N more", and a notice stays still while cells change height  · status: done

#### As-built

- A command cell keeps its last body row for `+{hidden} more` when its processes do not fit, at every width, full width (158 columns) included. `CommandRowBudget` (`render/frame_preparation.rs`) measures the rows with the same heading rule as the painter. `frame_preparation::hidden_count_row(groups, available_height, kind, hidden)` gives `prepare_table_viewport` the count row. A single-line command table also keeps the row, which `reserve_command_rows` skips.
- `tui_pane::render_toasts_over_grid(frame, framework, viewport, protected_frames)` (public, `overlays/frame_tail.rs`) draws the toasts over a tile grid through the crate-private `Toasts::render_over_grid`. cargo-tile calls it at one site, render.rs `draw`.
- `GridToastStack` (`toasts/render/drawing.rs`, field `Toasts::grid_stack`) is `Unplaced` or `Placed`. The first draw of a stack finds its rows. Later draws hold those rows while the viewport stays the same, no new toast joins, every card keeps its settled height, and no protected frame foot reaches the cards. A grid cell that changes height does not move a card, so the [x] hitbox matches the drawn [x] on every frame.
- Live on natedev: a full-width cell shows "+55 more". The startup notice holds one place through 20 cell height changes, and one click on [x] closes it.
- Tests: `a_full_width_command_cell_counts_its_hidden_processes` and `a_narrow_command_cell_counts_its_hidden_single_line_processes` (render.rs, through `assert_single_line_cell_counts_hidden_processes`); `a_drawn_toast_keeps_its_position_when_grid_frames_change_height` and `a_held_toast_moves_once_when_a_protected_foot_reaches_it` (`placement_tests.rs`, helper `GridNotice`); `one_click_closes_the_startup_notice_after_the_grid_changes_height` (`app_scenarios.rs`).

**Files:**
- `crates/cargo-tile/src/render.rs` — the notice call site and the count-row tests.
- `crates/cargo-tile/src/render/frame_preparation.rs` — `CommandRowBudget` and `hidden_count_row`.
- `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — the [x] click scenario.
- `crates/tui_pane/src/overlays/frame_tail.rs` — `render_toasts_over_grid`.
- `crates/tui_pane/src/toasts/render/drawing.rs` — `GridToastStack` and the hold rule; `toasts/manager.rs` holds the `grid_stack` field.
- `crates/tui_pane/src/toasts/render/placement_tests.rs` — the notice placement tests.

**Binds later work:** `a_toast_outside_the_summary_columns_keeps_its_original_rect` (render.rs) asserts the held place, one row above the settled place, with a comment that names Phase 74; the notice settle fix restores `assert_eq!(actual, expected)`. `GridNotice::draw` (`placement_tests.rs`) draws through `Toasts::render_over_grid`. `hidden_count_row` builds a second `CommandRowBudget` after `reserve_command_rows`.

**Gotchas:** A first draw that comes while the grid moves holds a place one row above the settled place for the whole life of the notice.

### Phase 2 — A notice settles once with the grid, and the count-row tests regain their strength  · status: done

#### As-built

- `tui_pane::render_toasts_over_grid` takes the `&TileGrid`. The grid reports a crate-private `GridStability::{Moving, Settled}` (`toasts/render/drawing.rs`), stored in `GridToastStack::Placed`. While the grid moves, `GridToastStack::next_visual_change_deadline` returns `At(now)`, the stack holds its first visible place, and the notice draws over a moving footer. On the first settled frame the notice moves once to its settled place. If the settled place cannot hold the full notice (`full_stack_fits`), the notice keeps its old place.
- `reserve_command_rows` builds one `CommandRowBudget`; `hidden_count_row` no longer exists. The count row is `HiddenProcessCount` (all fit, or some hidden with a `NonZeroUsize` count), and `hidden_process_count_label` is the one label formatter. `measured_table_row_use` counts the separator row.
- `TableSeparators::for_visible_groups` puts a cell's spare empty row between its last two directory groups, never directly above "+N more". A one-group cell lends its spare row to a column mate through the column allocator (`tui_pane/src/tiles/grid/allocation.rs`).
- Tests: `a_toast_outside_the_summary_columns_keeps_its_original_rect` asserts the settled place again; `a_toast_placed_while_the_grid_moves_moves_once_on_the_first_settled_frame`, `a_startup_notice_stays_visible_as_the_protected_foot_rises`, `a_startup_notice_keeps_its_place_when_the_settled_corner_is_too_short` and `a_startup_notice_first_drawn_short_keeps_its_place_during_motion` (`placement_tests.rs`); `near_full_summary_and_command_cells_leave_no_blank_rows_above_the_foot` and `a_command_cell_uses_its_spare_row_between_the_last_two_directory_groups` (render.rs) cover heights 53-67 with exact process and CPU-cell asserts.

**Files:**
- `crates/tui_pane/src/toasts/render/drawing.rs`, `toasts/render/mod.rs` — `GridStability`, the motion hold and the settled move.
- `crates/tui_pane/src/toasts/manager.rs`, `toasts/mod.rs` — the settled-frame deadline.
- `crates/tui_pane/src/tiles/grid/tile_grid.rs`, `overlays/frame_tail.rs` — grid motion state and the grid-aware call.
- `crates/cargo-tile/src/render.rs`, `render/frame_preparation.rs` — the row budget, `HiddenProcessCount`, `TableSeparators`.
- `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — startup scenarios draw a settled frame before they step deadlines.

**Gotchas:** `Toasts::next_visual_change_deadline` clamps each deadline to at least now + `FRAME_POLL_MILLIS` (8 ms). A test that steps notice deadlines must draw a settled frame first.

**Ruled out:** an empty row directly above "+N more" — the row split between the last two directory groups stays (user approved the look; showrunner ruling).
