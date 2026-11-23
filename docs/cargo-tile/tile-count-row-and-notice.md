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

#### Work Order

**Goal:** In a full-width cell with more processes than rows, the last row reads "+N more". A notice keeps its place and stays drawn while grid cells change height, and one click on its [x] closes it.

**Spec:**

Item 1 — the count row in a full-width cell.

1. Live evidence (natedev, 160x45 terminal): a fake `cargo clippy --workspace` with 60 child cargo processes. In a two-column layout its cell (r/c 28/78) ends with "+37 more". When the same cell is full width (r/c 20/158, content rows 63), no count row shows: the last process row ends with "…", and the tile footer comes next. Seen twice, with 30 and with 60 children.
   - Shot with the fault: `/session/tile/tile-validate/live/shots/c375_wide_no_count.png`.
   - Shot that works: `/session/tile/tile-validate/live/shots/c375_count.png`.
2. Likely cause: `prepare_table_viewport` sets `HiddenProcessCountRow::Reserved` only when `plan.body_rows_drawn < available_height`. The full-width plan seems to fill every row, so no row is left for the count. Trace the full-width case and fix the row budget at its cause.
3. The count row rules from earlier work stay: the label reads `+{hidden} more` in full when it fits, it uses the cell background, and it has its own last row.
4. Test: draw a command cell at full width (158 columns) through the live cell path, with more process rows than the cell height. Assert that the last body row reads `+N more` with the correct N. The test fails on the current code.

Item 2 — the notice during grid changes.

1. Live evidence: the startup notice "✓ Capturing cargo progress" sits at the bottom right of the screen. With no input, it hid for 0.1-0.2 s and slid in again 5 times in 12 s. Each time, a grid cell changed its content rows (other cargo jobs started or ended). The countdown ran 12 to 1 with no reset. A click sent while it slid missed [x] (two of three tries).
   - Trace: `/session/tile/tile-validate/live/c374_reslide_trace.txt`. Each line is the time in seconds and the right part of screen rows 31-45. "closing None xrow None" means that the notice was not on the screen.
2. Find what hides the notice and replays its slide when the grid changes. Candidates: a grid repaint over the overlay, a toast draw-area change, or a retained layer that drops the overlay. Fix it at its cause.
3. Wanted behavior: after its first entrance, a notice keeps its position and stays drawn on every frame while grid cells move, grow or shrink. Its countdown is not changed.
4. The [x] hit area matches the drawn [x] on every frame. One click on the drawn [x] closes the notice.
5. Tests:
   - Draw frames with a visible notice, change a grid cell's height between frames, and assert that the notice is drawn at the same place on each frame.
   - After the change, click the drawn [x] position once and assert that the notice closes.
   - Both tests fail on the current code.

Live check recipe for both items (scratch only; read the scripts before use):
- `/session/tile/tile-validate/live/observe2.py` records the notice position over time from a private tmux server `-L tilevalidate`.
- `/session/tile/tile-validate/live/fakebin/cargo` starts 60 child cargo processes in `/session/tile/tile-validate/live/proj`.
- Use only a private Xvfb display for shots, never `:0`. Stop every process you start.

**Files:**
- `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/render/` — the count row budget and its test.
- `crates/tui_pane/src/tiles/` — only if the count row cause is there.
- `crates/tui_pane/src/toasts/`, `crates/tui_pane/src/framework/` — the notice fix and its tests.
- `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — the notice scenario test.
- `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — one line for each fix.

**Workers:** 2 writers, split by item.
- `impl` — item 1: `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/render/`, `crates/tui_pane/src/tiles/`; hub: `crates/cargo-tile/CHANGELOG.md` (both items add one line; `impl` writes both lines).
- `impl2` (opens as impl) — item 2: `crates/tui_pane/src/toasts/`, `crates/tui_pane/src/framework/`, `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/shim_registration/app_scenarios.rs`, `crates/tui_pane/CHANGELOG.md`.

**State:** none

**Constraints from prior phases:**
- `docs/cargo-tile/as-built/tile-fixes.md` holds the earlier tile work. The count row came from the earlier "hidden-process row reads whole" work: `draw_hidden_process_count` clears its row and draws in `text_default()` on the cell background.
- Esc and the [x] close every notice (earlier work): `App::modal_key` gives Esc to a visible toast first; clicks go through `tui_pane::handle_tile_click`; `Framework` stores overlay bounds from the draw call.
- Toast entrance timing comes from `created_at` (`toast.rs`, `refresh_entrance_phase_at_width`).

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` passes, with the new full-width count-row test and the notice scenario test.
- `bash ~/.claude/scripts/direct/verify.sh test tui_pane` passes, with the new notice tests.
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile` and `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` are clean.
- Live on natedev: the full-width cell ends with "+N more"; the notice does not hide or slide while cells change; one click on [x] closes it.

### Phase 2 — A notice settles once with the grid, and the count-row tests regain their strength  · status: todo

#### Work Order

**Goal:** A notice that first shows while the grid moves keeps that place only until the grid settles, then takes its settled place once. The count-row tests prove their rules at every grid height again.

**Source:** showrunner ruling 2026-11-23 01:28 PST (Phase 73 lands after its 3 repair rounds; these items move here), and the round 3 review of Phase 1.

**Spec:**

Item 1 — the notice place after a moving first draw.
1. Fault: `a_toast_outside_the_summary_columns_keeps_its_original_rect` (render.rs). The first draw comes while the grid moves. `GridToastStack` keeps that place, one row above the settled place, for the notice's whole life. Phase 1 changed the assert to the held place, with a comment.
2. Fix: cargo-tile tells the notice draw whether the grid moves (an enum argument to `tui_pane::render_toasts_over_grid`). A place found while the grid moves is held only until the first settled frame. That frame searches once and holds the result. A place found on a settled frame stays held through later grid motion, as in Phase 1.
3. Restore the original assert `assert_eq!(actual, expected)` and remove the Phase 74 comment.
4. Keep every Phase 1 notice test green. The live fault (hide and slide in again while cells change height) must not return.

Item 2 — count-row test strength (round 3 review findings).
1. `a_command_cell_leaves_no_spare_row_while_a_column_mate_is_short`: test a range of heights (for example 53..=67), not one even height. Assert `repaired` is empty for every command cell.
2. `a_separator_loan_paints_the_recipients_new_process_rows`: assert that each recipient paints a pid that the former grid (`KeepTogether`) does not paint. Run it at an odd and an even height.
3. `nested_unrowed_descendants_add_to_their_nearest_drawn_row`: draw the parent and the child again (add a descendant, height 5), and assert the parent's own share and the child's sum, as main did.
4. `a_near_full_summary_yields_separators_for_hidden_process_rows`: also assert no blank rows in the command cells, and make height 63 pass too.
5. Derive the literals in the new count-row tests (45, 4115) from `TABLE_HEADER_HEIGHT`.

Item 3 — the lone blank row above "+N more" (known defect).
1. At width 200, roster [4,5,4,5,4,5], odd heights 57-67, one command cell keeps a blank row above "+N more" while its column mates are short. Cells grow in steps of 2 (heading and row), so one row has no use. Main has the same lone row at the other parity, without a count row.
2. A fix changes the look (for example, `TableSeparators::for_visible_groups` keeps the last `remaining` gaps, and `measured_table_row_use` models the same rule). A look change needs the user's approval: file the question before any code.

Item 4 — count-row code cleanups (round 3 review findings).
1. Build `CommandRowBudget` once: `reserve_command_rows` returns the hidden count and the count row; delete `hidden_count_row`; keep only the Summary arm in `prepare_table_viewport`.
2. Make `(AllFit, Reserved)` unrepresentable: one enum, for example `AllFit | SomeHidden { count_row }`.
3. Share one label function between `draw_hidden_process_count` and the test helper `has_blank_row_above_foot`, so a truncated label is also checked.
4. Model the separator that `TableSeparators::for_visible_groups` paints in the spare row in the single-line budget path, so measurement and painting agree on `kept`.

**Files:**
- `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/render/` — the notice call, the count-row code and the tests.
- `crates/tui_pane/src/overlays/frame_tail.rs`, `crates/tui_pane/src/toasts/` — the grid motion argument and the hold rule.
- `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — one line for each user-visible fix.

**Workers:** Claude subagents (no-worker mode): one for item 1, one for items 2 and 4. Item 3 waits for the user's answer.

**State:** none

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` and `bash ~/.claude/scripts/direct/verify.sh test tui_pane` pass, with the original toast assert restored.
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile` and `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` are clean.
- Live on natedev: the startup notice moves at most once, when the grid settles; it does not hide or slide while cells change.
