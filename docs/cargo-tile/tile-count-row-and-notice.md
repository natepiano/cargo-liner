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

#### Work Order

**Goal:** A notice that first shows while the grid moves keeps that place only until the grid settles, then takes its settled place once. The count-row tests prove their rules at every grid height again.

**Source:** showrunner ruling 2026-11-23 01:28 PST (Phase 73 lands after its 3 repair rounds; these items move here), and the round 3 review of Phase 1.

**Spec:**

Item 1 — the notice place after a moving first draw.
1. Fault: `a_toast_outside_the_summary_columns_keeps_its_original_rect` (render.rs). The first draw comes while the grid moves. `GridToastStack` keeps that place, one row above the settled place, for the notice's whole life. Phase 1 changed the assert to the held place, with a comment.
2. Fix: `tui_pane::render_toasts_over_grid` takes the grid, `grid: &TileGrid<Id>`. cargo-tile passes `&app.tiles` at its one call site (render.rs `draw`). A crate-private `TileGrid` query reports whether the grid moves: a step in flight or queued, the rule of `has_pending_motion_for_test`. Share that rule. The public function changes it into a crate-private enum argument of `Toasts::render_over_grid`, so tests pass the enum directly. No new public enum or `TileGrid` method (the name `GridMotion` is already used in cargo-tile terminal.rs and tui_pane motion.rs). A place found while the grid moves is held only until the first settled frame. That frame searches once and holds the result. A place found on a settled frame stays held through later grid motion, as in Phase 1. `GridToastStack::Placed` records which kind of frame found its place, as an enum field and not a bool. A search during motion after a protected foot reaches the cards also gives a held-until-settled place.
3. Restore the original assert `assert_eq!(actual, expected)` and remove the Phase 74 comment.
4. Keep every Phase 1 notice test green; `GridNotice::draw` (placement_tests.rs) passes a settled grid. Add one test in `placement_tests.rs`: a place found on a moving frame moves once on the first settled frame, then stays while cells change height. The live fault (hide and slide in again while cells change height) must not return.
5. The first settled frame is drawn at once. `TileGrid::tick` can report `Repaint::NotNeeded` on the settling frame (tile_grid.rs). While `GridToastStack` holds a place found during motion, `Toasts::next_visual_change_deadline` returns `At(now)`, so the event loop draws on the next poll after the grid settles.

Item 2 — count-row test strength (round 3 review findings).
1. `a_command_cell_leaves_no_spare_row_while_a_column_mate_is_short`: test the heights 53..=56 and the even heights 58..=66, not one height. For each height, assert that `repaired` holds no command cell. Build the former grid at one height only (each `row_report_comparison_with_view` builds two settled grids; keep the test under 1 s). Item 3 adds the odd heights 57..=67 with its fix.
2. `a_separator_loan_paints_the_recipients_new_process_rows`: assert that each recipient paints a pid that the former grid (`KeepTogether`) does not paint. Run it at an odd and an even height.
3. `nested_unrowed_descendants_add_to_their_nearest_drawn_row`: add a second descendant under the child and draw at height 5, so the parent and the child draw and the last row reads `+2 more`. Assert the parent's own share (10%) and the child's sum, as main did at height 4.
4. `a_near_full_summary_yields_separators_for_hidden_process_rows`: also assert no blank rows in the command cells at height 64. Height 63 is in item 3's odd range; add it with item 3's fix.
5. In `assert_single_line_cell_counts_hidden_processes`, derive 45 and 4115 from the cell height, `TABLE_HEADER_HEIGHT`, the heading row, the foot row and the count row.

Item 3 — the lone blank row above "+N more" (known defect).
1. At width 200, roster [4,5,4,5,4,5], odd heights 57-67, one command cell keeps a blank row above "+N more" while its column mates are short. Cells grow in steps of 2 (heading and row), so one row has no use. Main has the same lone row at the other parity, without a count row.
2. A fix changes the look: `TableSeparators::for_visible_groups` keeps the last `remaining` gaps, and `measured_table_row_use` models the same rule, so the spare empty row becomes the gap between the last two directory groups. The gap is an empty row, not a drawn line. The user approved this look change on the annotated before/after mockup (2026-11-23).

Item 4 — count-row code cleanups (round 3 review findings).
1. Build `CommandRowBudget` once: `reserve_command_rows` takes the table kind, builds the budget before its single-line early return, and returns the `HiddenProcessCountRow` that `hidden_count_row` returns now. Delete `hidden_count_row`. In `prepare_table_viewport`, a command table takes its count row from `reserve_command_rows`; the `visible_group_count > 0 && body_rows_drawn < available_height` rule stays only for `TableKind::Summary`.
2. Make `(AllFit, Reserved)` unrepresentable: one enum in `CommandRowBudget`, for example `AllFit | SomeHidden { count_row }`. Fold `PreparedTableViewport`'s `hidden` and `count_row` into one enum, for example `NoCountRow | CountRow { hidden: NonZeroUsize }`, and remove the `hidden > 0 && count_row == Reserved` filter in `draw_process_table`.
3. Share one label function between `draw_hidden_process_count` and the test helper `has_blank_row_above_foot`, so a truncated label is also checked.
4. In `measured_table_row_use`, the `CommandRowBudget` path that a single-line command table takes when it reserves its count row does not model the separator that `TableSeparators::for_visible_groups` paints in the spare row (`process_table_plan`). Model it there, so measurement and painting agree on `kept`. If this changes a cell height or a painted row, stop and add it to item 3's question.

**Files:**
- `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/render/` — the notice call, the count-row code and the tests.
- `crates/tui_pane/src/overlays/frame_tail.rs`, `crates/tui_pane/src/toasts/`, `crates/tui_pane/src/tiles/grid/tile_grid.rs` — the grid argument, the motion query, the hold rule and the settle repaint.
- `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — one line for each user-visible fix.

**Workers:** Codex workers: one for item 1 and its review; one implementer for items 4, 3 and 2 in that order (all touch render.rs).

**State:** none

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` and `bash ~/.claude/scripts/direct/verify.sh test tui_pane` pass, with the original toast assert restored.
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile` and `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` are clean.
- Live on natedev: the startup notice moves at most once, when the grid settles; it does not hide or slide while cells change.
