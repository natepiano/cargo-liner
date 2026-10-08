# tile-fixes

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-tile draws a moving cell on the painted ground, keeps every label and readout whole or marked, reads on a light terminal, shows the summary alone when the window is too small for cells, sizes the summary to what it asks, and nextest starts the slow tests first.

> **Production: remove-running-section** — unit `tile-fixes-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-08 06:40 PDT

now that we've added transparent - when a cell is being resized you can see (in this case with transparent off) something is happening that causes an artifact of making it looks like an oversized border around the resizing (probably removing) cell in cargo tile - i haven't seen this before and it looks odd

2026-10-08 06:5x PDT (relayed by the showrunner)

do the narrow column work - and for now just make sure slow tests run first in a cargo nextest run

2026-10-08 about 10:20 PDT (relayed by the showrunner; cargo-tile on the Mac, built from main 58bf6d15)

it shows mem in the column but it doesn't show summary mem - i just see hte mem label - oh - i was wrong - it's because it's white - and with light mode on my system i can't read it at all

2026-10-08 about 11:05 PDT (relayed by the showrunner; the first answers its question whether the summary sizing should change in this run)

don't worry about codex - you can add what you want to this

one thing though - cargo tile becomes useless when it is too small - the only way it is useful at narrow column width is just simply to show the summary table only

## Delegation Context

- **Project:** `cargo-tile` — a terminal grid of every running cargo command, one cell per command plus a summary cell. Its grid, borders and status line are drawn by `tui_pane`, which `cargo-handler` and `cargo-port` also build on.
- **Project started:** 2026-10-08T13:55:41.455+00:00
- **Worktree:** `/home/natepiano/rust/cargo-liner-tile-fixes`, branch `remove-running-section-tile-fixes`. Every seat works only here. Set by the showrunner (production doc, Units table).
- **Stack:** Rust workspace, ratatui 0.30.2, cargo-nextest (0.9.143 on natedev, 0.9.136 on the Mac).
- **Layout:**
  - `crates/tui_pane/src/pane/` — `frame.rs` (shared borders, `draw_clipped`, titles), `chrome.rs` (tint and screen ground).
  - `crates/tui_pane/src/tiles/` — `draw.rs` (the grid's drawing and the foot readout), `grid.rs` (arrangement, motion, `fits`), `constants.rs`.
  - `crates/tui_pane/src/bar/status_line.rs` — the status line every app draws.
  - `crates/cargo-tile/src/` — `render.rs` (cells, headings, ancestry, status line notes), `wrap.rs`, `constants.rs`.
  - `.config/nextest.toml` — test ordering, groups and timeouts for the workspace.
- **Key files:**
  - `crates/tui_pane/src/pane/frame.rs` — `PaneFrame` (66), `draw_clipped` (196), `fill_pane` (229), `blit` (237), `GridLines::add` (436), `GridLines::add_titled` (480), `overlay_row` (599), `written_row` (642), `write_overlay` (661), `mark` (670), tests from 761 (`a_title_stops_inside_the_corner` 1071, `a_shifted_pane_keeps_only_what_lands_in_its_clip` 1132).
  - `crates/tui_pane/src/pane/chrome.rs` — `pane_fill` (129), `screen_ground` (145), `pane_background` (157).
  - `crates/tui_pane/src/tiles/draw.rs` — `draw_tile_grid` (144), `draw_placements` (168), `has_visible_body_row` (263), `draw_cell` (310), `draw_rows_readout` (401), `draw_rows_readout_after_foot` (407), `rows_readout_line` (428), `draw_rows_readout_line` (461), `draw_summary_foot` (469), `content_area` (495), `readout_area` (513), tests from 542 (`StubCells` 551, `draw_motion_fixture` 635, `settled_grid` 668).
  - `crates/tui_pane/src/tiles/grid.rs` — `TileDrawing` (167), `TileGrid` (323), `sync` (549), `queue_with_depth` (777), `progress` (1040), `placements` (1053), `drawing` (1059), `Grid::new` (1275), `columns` (1496), `fits` (1649), `shared_run` (1756), `Turns` (1773), `moving_cell` (1867), `wrapping_cell` (1921), `column_band` (1979), `lerp_rect` (2079), tests from 2140.
  - `crates/tui_pane/src/tiles/constants.rs` — `MIN_TILE_HEIGHT` (18), `MIN_TILE_WIDTH` (21), the `TILE_ROWS_*` readout constants.
  - `crates/tui_pane/src/bar/status_line.rs` — `render` (162), `status_line_note_spans` (205), `status_line_global_spans` (222), `render_sections` (272). No test module yet.
  - `crates/cargo-tile/src/render.rs` — `draw` layout (238), `Cells` and its `TileCells` impl (312), `summary_width` (438), `sccache_label` (716), `draw_ancestry` (939; truncation at 983), `ancestry_stem` (1130), `ancestry_room` (1144), `ancestry_rows` (1159), `ancestry_lines` (1180), `PathGroup::heading` (1373), `draw_path_group` (1634), `process_row` (1827), `heading_gauge` (2051), `cell_width` (2180), `draw_status_line` (2187), tests from 2300 (`buffer_line` 2419, `narrow_table_buffer` 4801, `buffer_rows` 5510).
  - `crates/cargo-tile/src/wrap.rs` — `Wrap::push` (72), `wrapped` (118), `split_at_cells` (151), tests from 160.
  - `crates/cargo-tile/src/constants.rs` — `ACCOUNT_HEADING_OPEN`/`CLOSE` (7, 9), `SUMMARY_CELL_TITLE` (149), `ANCESTRY_ELISION` (169), `TABLE_NO_COLUMNS_MARKER` (427).
  - `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — whole-frame test helpers `draw` (317), `draw_settled` (594).
  - `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/README.md` — user docs; each changelog has an `## [Unreleased]` section.
  - `.config/nextest.toml` — two priority overrides (100 and 90), group `cargo-tile-readers`, two `slow-timeout` overrides.
- **Test lanes:** `cargo-tile` — `crates/cargo-tile/tests/` (`unit_tests.rs` compiles every `src` module with its `#[cfg(test)]` tests into the binary `cargo-tile::unit_tests`). `tui_pane` — `crates/tui_pane/tests/`; frame, tile and status-line tests live in-module because they read private items. `cargo-handler` — none.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - A value on screen is whole, or shortened with a `…` that marks the cut, or absent. Nothing is cut without a mark (showrunner's gate for this unit: nothing clipped mid-value).
  - A command cell is never narrower than 40 cells in cargo-tile. A window that cannot hold the grid at that width shows the summary alone, filling the window (user, 2026-10-08: cells are useless when too small).
  - Every literal the code names lives in its crate's `constants.rs` (existing crate convention).
  - No test takes a second. Render tests are pure and run on buffers; none waits on the clock (user rule, memory `no-multi-second-tests`).
  - A `tui_pane` change reaches `cargo-handler` and `cargo-port`; a gate that touches `tui_pane` also runs `verify.sh test` for both (production doc, Merge tests). No existing `tui_pane` public signature changes, and `cargo-handler`'s cell sizes do not change (author's calls).
  - Format with `cargo +nightly fmt` only (user rule, memory `rustfmt-nightly-only`).
  - No UX guide covers cargo-tile (production doc, Production rules): design checks judge by the three gods in `~/.claude/docs/decision_criteria.md`.
  - Captures are the unit director's, taken before each checkpoint notice with a design check by a fresh helper (showrunner's gate). Screens are pty captures of the real `target/debug/cargo-tile tile`, replayed with pyte and split into frames on `\x1b[?25l`; the settled screen is the last completed frame, never a byte-offset sample.

## Phases

### Phase 1 — A cell in flight stands on the painted ground  · status: done

#### As-built

- A piece in flight is drawn on the screen ground. `draw_clipped` lays `chrome::screen_ground()` over the whole scratch buffer before `fill_pane`, so the ring sits on the ground and the tint inside it; `blit` keeps a piece's interior rows off `clip.top()` and `clip.bottom() - 1` and still copies its ring rows there. With `transparent = true`, `screen_ground()` is `None` and every cell keeps `Color::Reset`.
- `TileGrid::drawing(area, growth) -> TileDrawing<Id>` is crate-private (`pub(super)`) and carries `placements` and `column_bands` from one clock reading. The public `TileGrid::placements` returns `self.drawing(area, growth).placements`.
- `draw_tile_grid` calls the private `draw_placements`, which takes a `&TileDrawing<Id>` and draws every column band as a fixed frame each frame.
- `GridLines::add` marks the clip's border row across a pane the clip cuts.

**Files:**
- `crates/tui_pane/src/pane/frame.rs` — `draw_clipped`, `blit`, `GridLines::add`, render tests
- `crates/tui_pane/src/tiles/draw.rs` — `draw_placements`, `has_visible_body_row`, render tests
- `crates/tui_pane/src/tiles/grid.rs` — `TileDrawing`, `TileGrid::drawing`
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — Fixed entry

**Binds later work:**
- `TileDrawing` is a one-frame snapshot (`placements`, `column_bands`) returned by `TileGrid::drawing` and consumed only by `draw_placements`. All grid drawing runs `draw_tile_grid` → `draw_placements(buffer, area, &drawing, &demands, &widths, contents, cells)`.
- A piece with no body row in view is not drawn (`has_visible_body_row`; test `an_edge_only_piece_adds_no_line_above_the_column_frame`).
- Lines carry the screen ground as background while it is painted, and keep the default with transparent on or contents hidden (test `a_line_over_an_overlapped_cell_sits_on_the_screen_ground`).
- A pane's side lines end on its clip's top and bottom rows (`GridLines::add`; the `frame.rs` clip-row tests).

**Gotchas:**
- Motion is captured as the frames the app writes to a pty, replayed with pyte and rendered to PNG: the session is Wayland and the terminal window cannot be recorded. A design check on motion uses frames 0.2 to 0.5 s into each motion; the first frame of a motion equals the settled frame before it.
- A machine job starting or ending during a capture adds a motion of its own; a frame is settled only 1.8 s after the last key.
- `TileGrid::progress` reads the clock, so a test cannot drive `drawing` through exact steps.

**Ruled out:** renaming `TileDrawing` and `drawing` (the name says what it holds, and "frame" collides with `PaneFrame`); a `TransitionProgress` newtype (the file's motion helpers all take the `u32` scale); a second readout state type for candidate fit (the choice stays inside the drawing function); a tester that owns no file running the motion capture (the capture is the unit director's).

### Phase 2 — Touching cells and columns share one line through a motion  · status: done

#### Work Order

**Goal:** While cells open, close and change columns, two cells that touch share one separator and two columns share one divider in every frame, as they do when the grid is still.

**Spec:**

Follow-up to Phase 1, written under the production's hard landing rule: Phase 1's last design check (2026-10-08, 47 frames of a 200x50 pty capture, ten `+`/`-` motions) raised six rows that all predate Phase 1. Measured over every frame in the first second of each motion, before Phase 1 → after it: a line doubled on the grid's top or bottom frame row 8.1% → 8.4%; two separators on adjacent rows between two cells 37.2% → 27.9%; two vertical lines side by side between columns 12.0% → 1.5%. The gate below takes all three to zero.

Cause, one for all three: settled neighbours overlap on one inclusive frame line (`upper.bottom() - 1 == lower.top()`, `left.right() - 1 == right.left()`). In a motion each rect is worked out alone: `moving_cell` (`grid.rs:1867`) interpolates a resident, arriving or departing cell with `lerp_rect` (2079); `column_band` (1979) does the same per band, an absent column being a zero-width rect on the exclusive `area.right()`; `wrapping_cell` (1921) truncates its two pieces' shifts separately with `travel`. Nothing keeps the shared line shared, so it shows as two adjacent rows or columns.

0. **A pure entry point.** `TileGrid::progress` (`grid.rs:1040`) reads the clock, so no test can drive `drawing` (1059) through exact steps. Extract `pub(super) fn drawing_at(&self, area: Rect, growth: TileGrowth, raw: u32) -> TileDrawing<Id>`, `raw` on the `PROGRESS_SCALE` scale before easing, as the file's motion helpers take it; `drawing` reads the clock once and delegates. `placements` and `TileDrawing`'s two fields are unchanged, so `draw_placements` (`draw.rs:168`) needs no change for this.
1. **Columns share their dividers.** `Grid::columns` holds overlapping rects, not divider positions. In `drawing_at`, derive each end's divider vector as `d0 = first.left()` followed by every band's `right() - 1`, pad the shorter vector with `area.right() - 1`, interpolate each divider once, and form band `k` as the inclusive span `dk..=d(k+1)`. An opening or closing column therefore collapses onto the last drawable column, not today's zero-width rect on exclusive `area.right()`; a one-cell band there re-adds the outer line and nothing else. Keep today's per-band top and bottom interpolation for the columns below a widened summary. At every step the first divider is `area.left()`, the last is `area.right() - 1`, and each band's right divider is its neighbour's left. Covers the bare `│` beside the frame and the `┐┌` pair beside an opening or closing column.
2. **One row topology per column band.** Replace the `(Option<usize>, Option<usize>)` pair taken by `moving_cell` and `Turns::covers` (1833) with `CellTransition::{PresentAtBothEnds { before_cell, after_cell }, Arriving { after_cell }, Departing { before_cell }}`; `(None, None)` is not a state. Resolve each transition into the ordered pieces of its band, `BandPieceMotion::{Resident, Entering { edge }, Leaving { edge }}`, and interpolate the band's inclusive divider rows once: a cell growing or shrinking in place moves one divider for both neighbours; an arrival or departure collapses on `band.top()` or `band.bottom() - 1`; a cell crossing columns stays one leaving and one entering `PaneFrame::shifted` piece, each taking its place in its own band's order. Build each piece from two consecutive dividers; the first and last boundaries are the band's own frame rows. Keep transition order, never sort interpolated rects: `sync` reorders only by close then open, which `no_step_of_a_reorder_draws_two_cells_crossing` pins. `edge_rect` (2004) and `closing_rect` (2038) go or shrink to what the topology still needs. Both new types are private to `grid.rs`.
3. **No ground-only row between neighbours.** With the ground painted, every row between two neighbouring visible bodies is their one shared separator, never bare ground (design check: rows 44 to 47 under a closing job's cell, rows 9 to 24 of a new column). This is item 2 seen on screen: add no second mechanism. A piece collapsed to its border draws nothing (`has_visible_body_row`, `draw.rs:263`, pinned by `an_edge_only_piece_adds_no_line_above_the_column_frame`); that stays.
4. **A queued step never sends a cell through a third column.** In the capture a job's cell went to the foot of the left column for 0.37 s, then back to the head of the middle column where it started, while the summary was squeezed. 0.37 s is one of two queued steps (`animation_millis` 720 over two). `queue_with_depth` (777) queues a summary-depth change as its own step before or after the slot steps, and `Grid::new` (1275) lays columns out from the cell count plus `summary_depth - 1`, so a depth-only step can re-divide the columns and the next step put them back. Reproduce it first: a grid of three columns with a job at the head of the middle one, then one `sync` or `+` whose queue holds a depth step and a slot step; walk every queued `(slots, depth)` grid. Rule: at every queued step each surviving cell stands in the column it has at the start of the queue or the one it has at its end. Where a depth-only step breaks the rule, fold that depth change into the slot step beside it; other queues keep today's separate steps. Also pin the in-place handover, which already holds: `sync` during an opening leaves the transition in flight untouched and the claimed cell never appears in a second band. If the reproduction shows another cause, fix that cause in `grid.rs` and say so in the summary.

Tests, all pure, through `drawing_at` at 24 evenly spaced `raw` values per motion; reuse `grid.rs`'s `test_area`, `even`, `quiet`, `add_new`, `redistribute`, `drawn`, `seeded_grid` and `draw.rs`'s `StubCells`, `area_lines`, `buffer_line`, `is_lattice_glyph`, `draw_motion_fixture`:
- `grid.rs`: `column_bands_tile_the_area_through_a_transition` — an opening and a closing column: item 1's three statements at every step.
- `grid.rs`: `pieces_in_a_column_share_their_separators_through_a_transition` — opening, closing, growing in place and a cell crossing columns: in every band each piece's bottom row is the next piece's top row, the first and last boundaries are the band's frame rows.
- `grid.rs`: `a_queued_depth_step_keeps_every_cell_in_a_column_it_starts_or_ends_in` and `a_cell_claimed_during_an_opening_stays_in_its_transition_column` (item 4).
- `draw.rs`: `no_line_is_doubled_through_a_transition` — render those snapshots with `draw_motion_fixture` and read the buffer: no `───` on two adjacent rows at the same columns, no pair matching `[│┤┐┘][│├┌└]` on any row.
- `draw.rs`: `a_collapsing_cell_leaves_no_ground_only_row_between_neighbours` (item 3), opening and closing.

Changelog: extend the `## [Unreleased]` Fixed entry in `crates/tui_pane/CHANGELOG.md` and `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `drawing`, `drawing_at`, `queue_with_depth`, `Turns::covers`, `moving_cell`, `wrapping_cell`, `column_band`, `edge_rect`, `closing_rect`, `CellTransition`, `BandPieceMotion`, tests.
- `crates/tui_pane/src/tiles/draw.rs` — render tests; `draw_placements` or `has_visible_body_row` only if a render test proves a change is needed.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — Fixed entry.

**Seats:** 1 writer + 1 tester — the geometry is one hand in `grid.rs`; the render tests are a real lane in `draw.rs`, written against this Spec while the geometry is built.
- `impl` — `crates/tui_pane/src/tiles/grid.rs` (types, geometry, queue, its in-module tests), both changelogs. Lands `drawing_at` first and posts it on the board.
- `test` — `crates/tui_pane/src/tiles/draw.rs`: the two render tests, written against items 1 to 3 and failing until `impl` lands them; any change to `draw_placements` they prove necessary.

**Constraints from prior phases:** Phase 1 added the crate-private `TileDrawing` snapshot (`placements`, `column_bands`) returned by `TileGrid::drawing`, drew every column band as a fixed frame in `draw_placements`, and stopped a pane's side lines at its clip's top and bottom rows in `GridLines::add` (`frame.rs`). The drawing path is `draw_tile_grid` (`draw.rs:144`) → `draw_placements(buffer, area, &drawing, &demands, &widths, contents, cells)` (168). `frame.rs` does not change here. `TileDrawing` keeps its name and fields. No new public item; no `#[allow]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's capture as in Phase 1 (200x50, five `+`, five `-`, transparent off and on), every frame in the first second of each motion: no line doubled on a frame row, no two separators on adjacent rows, no two vertical lines side by side; the Phase 1 measures still zero (no unpainted cell with transparent off, no line off the ground, no stub out of the grid).
- A fresh helper's design check of PNG renders of those frames, each motion frame 0.2 to 0.5 s in: pass.

### Phase 3 — A closing column empties in one motion  · status: todo

#### Work Order

**Goal:** When a step removes a column, the cells in it leave without any of them growing and without a cell's title showing twice.

**Spec:**

Follow-up to Phase 2 under the production's hard landing rule. Phase 2's last design check (44 frames, fourteen `+`/`-` steps at 200x50) passed every line, background and arrival state and raised this one row on style. In three close steps where the last column goes away (`ph2d_off_11_close_motion`, `ph2d_on_09_close_motion`, `ph2d_on_13_close_motion`): the cell moving to the neighbour column is drawn in the closing column as a whole shortened cell, title on its top row and readout on its bottom row, while it is also arriving in the neighbour column with its title; and the cell being removed grows under it (22 to 32 rows). Where the column stays, a leaving cell slides off the top and shows only its last rows.

Cause as read; reproduce it in a render test first. `wrapping_cell` (`grid.rs`) builds the piece left in the old column as `BandPieceMotion::Leaving { sliding: from_column < after.widths.len(), .. }`, so it does not slide when its column is absent from the after grid: `piece_frame` draws it as a plain `PaneFrame::new(rect)` whose rows collapse, and the departing cell below takes those rows.

Rule (unit director's default; the showrunner owns the look and may replace it before dispatch):
1. A column the step removes closes as one: every piece in it keeps the rows and the contents it has at the start of the step, nothing in it moves up or down or changes height, and the band narrows onto its divider as Phase 2 built it.
2. A cell's title is drawn once: on its entering piece. A `Leaving` piece, in a closing column or a staying one, adds its lines without a title.
3. Phase 2's measures hold in every frame: no doubled line, no interior row on the screen ground, no blank slot, no cell in a third column.

Tests, pure, 24 snapshots per step through `drawing_at`, with `draw_motion_fixture`:
- `draw.rs`: `a_title_is_on_screen_once_while_its_cell_changes_columns` — a column closing and a column staying.
- `grid.rs`: `no_piece_in_a_closing_column_changes_height` — every piece's rows at every snapshot equal its rows at the start.
- Phase 2's motion and render tests pass unchanged; a test that pinned the old shape is changed only for this rule and named in the summary.

Changelog: extend the `## [Unreleased]` Fixed entry in both changelogs.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `wrapping_cell`, `place_band_pieces`, `band_piece_dividers`, `piece_frame`, tests.
- `crates/tui_pane/src/tiles/draw.rs` — `draw_placements` (title only on a piece that is not leaving), render test.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — Fixed entry.

**Seats:** 1 writer + 1 tester — the geometry is one hand in `grid.rs`; the render test is written from this Spec in `draw.rs`.
- `impl` — `crates/tui_pane/src/tiles/grid.rs`, both changelogs; takes `draw.rs` for the title rule once `test` posts `done`.
- `test` — `crates/tui_pane/src/tiles/draw.rs`: the render test, failing until `impl` lands the rule.

**Constraints from prior phases:** Phase 2 made `TileGrid::drawing_at(area, growth, raw)` the pure entry to a motion, resolved each cell into `BandPieceMotion::{Resident, Entering, Leaving}` pieces that share their dividers per column band, and shifts a sliding piece onto the edge of its clip (`shift_to_clip`). `TileDrawing` keeps its fields. No new public item; no `#[allow]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's capture as in Phase 2 (200x50, seven `+`, seven `-`, transparent off and on): Phase 2's measures all zero, and in each step that removes a column no piece in it changes height and no title shows twice.
- A fresh helper's design check of those frames: pass.

### Phase 4 — Headings, titles and readouts are whole or marked, and text reads on a light terminal  · status: todo

#### Work Order

**Goal:** At every cell width a directory heading, a border title and the foot readout are drawn whole, shortened with `…`, or left out — never cut bare; and every value in the summary and the cells can be read on a light terminal as well as a dark one, with transparent on and off.

**Spec:**

1. **Directory heading** (`cargo-tile`, `draw_path_group`, `render.rs:1634`). Today `PathGroup::heading()` is one span at natural width in an unwrapped `Paragraph`, cut at the cell edge (`[natep`). Add `PathGroup::fitted_heading(&self, room: u16) -> String` and draw that; `room` is `area.width` less `SECTION_HEADER_INDENT`. It returns the first of these that fits `room` (measured with `cell_width`):
   1. the whole heading, `[account] path` (or `path` alone for an unqualified group);
   2. `[account] …/rest`, dropping whole leading path components, keeping as many trailing components as fit;
   3. `[account] …tail`, the last component cut from its head, while at least `HEADING_MIN_TAIL` (8) cells of it show;
   4. forms 2 then 3 again without the `[account] ` prefix;
   5. the empty string.

   A shorter `…tail` form is not built: Phase 5 keeps every command cell at 40 cells or wider, where form 4 always fits.

   The project name is the end of the path, so the end is what survives. `…` is `ANCESTRY_ELISION`'s glyph; name a `HEADING_ELISION` constant for it. `heading_gauge` and `summary_width` keep measuring the whole heading: the gauge already goes before the heading sheds anything, and `widen_summary` asks for the whole width.
2. **Border title** (`tui_pane`, `write_overlay`, `frame.rs:661`). A title wider than its row is cut bare (` summar`, `┌ Tar┐`). For `OverlayStyle::Title`, when the text is wider than `row.width`, draw its first `row.width - 1` cells and then `…`; a row one cell wide draws `…` alone. `written_row` already records the drawn width. Labels (`OverlayStyle::Label`) are whole or absent through `clear_run` and do not change. Name the glyph `TITLE_ELISION` in `crates/tui_pane/src/pane/constants.rs`. `a_title_stops_inside_the_corner` now expects `┌ Ta…┐`. Measure with `unicode_width`, as the file does.
3. **Foot readout** (`tui_pane`, `draw.rs`). `readout_area` clamps the line to the room and the `Paragraph` cuts it (`conten`, `r/c: 22/2`). `rows_readout_line` becomes `rows_readout_lines(inner, rows, measured_at) -> Vec<Line<'static>>`, widest first:
   1. the whole line as today: `content rows: N[ @ W]  r/c: H/W`;
   2. `content rows: N @ W` — only when `measured_at != inner.width`;
   3. `content rows: N`.

   `draw_rows_readout` draws the first candidate whose width is at most `inner.width - TILE_ROWS_RIGHT_INSET`, right-aligned as now, and nothing when none fits. `draw_rows_readout_after_foot` runs the same list against the room left after the summary foot. The readout row stays reserved either way, so no cell changes height. One exception, from Phase 2's last design check: a cell with a single body row drew `content rows: 0  r/c: 1/49` and no label, so it had no name. A cell with one body row gives that row to its contents (its label) and draws no readout; `readout_row` returns `CellTooSmall` there. Today that reservation rests on the clamp: `content_area` (`draw.rs:495`) asks `readout_area(inner, inner.width) -> Option<Rect>` (513), which clamps the width to the room. So separate the two: replace `readout_area` with `readout_row(inner) -> ReadoutRow`, `ReadoutRow::{Reserved(Rect), CellTooSmall}`, the whole last interior row inside the right inset. `content_area` reads only that. The drawing functions pick the first candidate no wider than the reserved row's room and right-align it there; no candidate is ever clamped, and no readout function returns a bare `Option<Rect>`.
4. **Text reads on a light terminal** (user, 2026-10-08, cargo-tile on a Mac in light mode: the summary's memory total could not be read, "it's because it's white"). The total's value is drawn in `text_default()` (`render.rs:230`, in `SummaryMemoryTotal::foot`), the theme's default text colour, `Color::White` in the fallback theme (`crates/tui_pane/src/theme/fallback.rs:65`); the `mem ` label beside it is `label_color()` (gray) and reads. `text_default()` (`crates/tui_pane/src/theme/accessors.rs:100`) has 72 call sites across the three apps, six in cargo-tile's `render.rs` (230, 961, 1816, 1906, 1917) and two in the foot readout (`tiles/draw.rs:456`, `459`).
   - Reproduce first, in a pure render test: draw the summary foot with `transparent_background()` on, then off, and read the value's foreground. State in the summary which case puts white on the terminal's own background.
   - Rule: default text stands on one of two grounds. On the painted screen ground (`chrome::screen_ground()`, transparent off) it keeps the theme's colour, which the theme pairs with that ground. With transparent on nothing is painted under it, so it takes the terminal's own foreground, `Color::Reset`, which the terminal pairs with its own background, light or dark.
   - Where: `text_default()` itself, so one change reaches every caller in all three apps. Before changing it, list its callers that blend or convert the colour (`blend_color`, `Layout::ink` at `render.rs:1476`, the RGB conversion at `chrome.rs:179`): each must still produce a readable ink on both grounds with transparent on. A caller that needs a concrete colour reads the theme's colour through a crate-private accessor; add a public item only if a caller outside `tui_pane` needs one, and name it in the summary.
   - Audit cargo-tile's summary and cells for any other ink that assumes a dark ground with transparent on (a literal `Color::White`, a `bright` or `default` theme colour used as a foreground, a fade that blends toward a ground that is not painted). Fix each by the same rule and list them in the summary.

Tests, all pure:
- `accessors.rs` or `render.rs`: `default_text_takes_the_terminals_foreground_on_a_transparent_screen` and `default_text_keeps_the_theme_colour_on_the_painted_ground`; `render.rs`: `the_summary_memory_total_reads_on_either_ground`.
- `render.rs`: `every_heading_width_is_whole_marked_or_absent` — for each width from 0 to the whole heading's width, for a qualified and an unqualified group: the drawn heading is the whole heading, or contains exactly one `…` and ends with a suffix of the path, or is empty; and it never exceeds the width. One case pins `[natepiano] …/tool-based-ui-frame-time`.
- `frame.rs`: the updated `a_title_stops_inside_the_corner`, and `a_title_one_cell_wide_is_the_mark_alone`.
- `draw.rs`: `the_rows_readout_is_a_whole_candidate_at_every_width` — for each inner width from 0 to 60: the readout row, trimmed, is empty or equals one of the candidates exactly. Update `draw_tile_cell_draws_only_the_readout_on_its_foot_row` only if its expected text changes (it should not).

Changelogs: one Changed line each under `## [Unreleased]`.

**Files:**
- `crates/cargo-tile/src/render.rs` — `PathGroup::fitted_heading`, `draw_path_group`, test.
- `crates/cargo-tile/src/constants.rs` — `HEADING_ELISION`, `HEADING_MIN_TAIL`.
- `crates/cargo-tile/CHANGELOG.md` — Changed line.
- `crates/tui_pane/src/pane/frame.rs` — `write_overlay`, tests.
- `crates/tui_pane/src/pane/constants.rs` — `TITLE_ELISION`.
- `crates/tui_pane/src/tiles/draw.rs` — readout candidates, test.
- `crates/tui_pane/src/theme/accessors.rs` — `text_default`, tests; any `tui_pane` caller the audit names.
- `crates/tui_pane/CHANGELOG.md` — Changed line, Fixed line.
- `crates/cargo-handler/src/**`, `crates/cargo-port/src/**` — only a test that pinned white default text on a transparent screen; name each in the summary.

**Seats:** 2 writers — the work splits by crate.
- `impl` — `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/pane/constants.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/theme/accessors.rs`, `crates/tui_pane/CHANGELOG.md`, and any `cargo-handler` or `cargo-port` test the colour rule changes.
- `test` — opens as impl: `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/CHANGELOG.md`.

**Constraints from prior phases:** Phase 1 split `draw_tile_grid` into itself plus a private `draw_placements` in `draw.rs`, and changed `draw_clipped` and `blit` in `frame.rs`; neither touches titles or readouts. Phase 2 changed how `grid.rs` places moving pieces and column bands; `draw.rs` gained render tests beside the readout code and no readout change. The drawing path is `draw_tile_grid` (`draw.rs:144`) → `draw_placements(buffer, area, &drawing, &demands, &widths, contents, cells)` (168).

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port`, `... lint tui_pane` and `... lint cargo-tile` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures beside a real `cargo check`, 50 rows, at 200, 126, 90, 64, 48 and 40 columns: no heading, title or foot readout is cut bare in any of them.
- The 126-column capture rendered four ways: on a dark and on a light terminal (white background, black foreground), each with transparent off and on. Every value in the summary and the cells is readable in all four; the memory total is named in the notice.
- A fresh helper's design check of those ten shots. Headings, titles, readouts and light-terminal text must pass; defects in ancestry text, the status line and cells under 40 wide are recorded for Phase 5 and do not fail this gate.

### Phase 5 — Ancestry, status line, a floor for cells, and the summary alone below it  · status: todo

#### Work Order

**Goal:** Ancestry text breaks at path and word boundaries, the status line drops whole items as the row narrows, cargo-tile never makes a command cell narrower than 40 cells, and a window too small for the grid shows the summary alone, filling the window.

**Spec:**

1. **Wrap at boundaries** (`cargo-tile`, `Wrap::push`, `wrap.rs:72`). A word wider than a whole line is cut today at a raw cell count. New rule for that branch: break after a boundary character — any of `WRAP_BREAK_AFTER = "/-=_.:,"` — taking the last boundary that fits the room left on the current line; with none in that room and the line not empty, wrap first and try a whole line; with none in a whole line either, cut at the line's width as today. Words that fit a line are untouched. `wrapped` serves the ancestry chain and the table's command column, and demand and draw both call it, so they stay in step.
2. **A cut level says so** (`draw_ancestry`, `render.rs:983`). `lines.truncate(budget.max(1))` drops a level's last lines bare. When it drops any, the last kept line ends in `ANCESTRY_ELISION`: appended when the line has a free cell inside `ancestry_room`, otherwise replacing its last cell.
3. **No room, no command** (`ancestry_stem`, `ancestry_room`, `render.rs:1130`, `1144`). When a level's room for its command is under `ANCESTRY_MIN_COMMAND_WIDTH` (8), the level draws its pid alone on one row; `ancestry_rows` counts it as one row. The pid stays whole or absent as now.
4. **Status line fit** (`tui_pane`, `status_line.rs`, `render` at 162 and `render_sections` at 272). Nothing is measured today: the right block is right-aligned, starts at column 0 when it is wider than the row, and its tail (`? shortcuts`) is cut. New order, whole items only:
   1. the globals (` ? shortcuts`) are kept while they fit the row with their one trailing cell;
   2. notes are added in front of the globals from the last note backwards while each fits whole — so the app's name and version, the first note, go first;
   3. the left segment (uptime, then navigation) is drawn only when it fits whole in the room left of the right block with one cell between; its navigation spans go before the uptime does;
   4. the centre is drawn only when it fits whole between them.

   Add a test module to `status_line.rs`; build the spans with the file's own helpers.
5. **A floor where cells stop being useful** (user, 2026-10-08: "cargo tile becomes useless when it is too small"). The framework minimum (`MIN_TILE_WIDTH`, 8) stays as it is, because `cargo-handler` draws its agent cells on the same grid and its narrow cells are its own design. Add `TileGrid::set_min_tile_width(&mut self, width: u16)` in `tui_pane` (it writes `settings.min_tile_width`, never below the framework minimum), and have cargo-tile call it once where it builds its grid (`app.rs:207`) with a new `MIN_CELL_WIDTH = 40`: 38 cells inside hold a whole `[account] project` heading of ordinary length, a pid with a 28-cell command, and the whole foot readout; an 80-column terminal holds two columns. `fits` (`grid.rs:1777`) already refuses to open a cell that would make columns narrower — on `+`, on a command arriving, on `sync` — so in cargo-tile five columns need 196 terminal columns, four 157, three 118, two 79, one 40. A command refused a cell is still counted in the summary, as today.
6. **Too small shows the summary alone** (user, 2026-10-08: "the only way it is useful at narrow column width is just simply to show the summary table only"). Add `TileGrid::holds_in(&self, area: Rect, growth: TileGrowth) -> bool`, crate-private (`pub(super)`): the `fits` test applied to the arrangement the grid is headed for. It counts every position that arrangement lays out, `target().len() + TABLE_CELL + target_depth().saturating_sub(1)`, because `Grid::new` adds the summary's depth before it divides the columns; `fits` given the cell count alone accepts a deep-summary grid that does not fit. In `draw_tile_grid` (`draw.rs:144`), after `grid.sync`, when it is false and contents are shown: draw the summary cell alone, its frame on the whole `area`, through the same path a settled summary cell takes in `draw_placements` and `draw_cell` (title, labels, foot, readout, ground and tint as any cell), with no column band and no command cell, and return. The summary's demand and its measured width come from the whole area's inner width in that frame, not from the arrangement's column. There is no motion into or out of this state: the frame after a resize is the summary alone or the settled grid. The grid keeps syncing and keeps its cells, so they stand in their settled rects in the first frame whose area holds them; a command refused a cell while the window was small opens one by the ordinary `sync` rule once there is room. Contents hidden: nothing is drawn, as today. An area too small for a frame with one body row and one body cell draws nothing. The same rule serves `cargo-handler` at its own 8-cell minimum, where today it draws border-only slivers. This replaces the earlier plan's `window too small` notice: no `TILE_GRID_TOO_SMALL` constant and no surface type are added.
7. **The summary reads at every width** (`cargo-tile`, `draw_summary`, `render.rs:611`). Alone in a narrow window the summary is all the user has. Its table already sheds columns (`TABLE_NO_COLUMNS_MARKER`, `narrow_table_buffer` at `render.rs:4801`). Pin it: at every inner width from 6 to 38, each header, each table value and the foot is whole, shortened with `…`, or absent. Fix in `draw_summary` whatever the test finds cut bare, by the rules Phase 4 set (the end of a path survives; a value is never cut without its mark).

Tests, all pure: `wrap.rs` — a long path breaks only after `/`, a flag after `=` and `-`, a run with no boundary still cuts at the width; update the tests that pin today's cuts (`wrap.rs` 219, 227; `render.rs` 3112, 3140). `render.rs` — a truncated level ends in `…`; a level under the minimum room draws its pid alone; demand equals draw at widths 22 to 60. `status_line.rs` — at every width from 0 to the full row's natural width, each item's text appears whole or not at all, and the globals outlast the notes. `grid.rs`/`draw.rs` — with the minimum set to 40, `fits` holds at 196, 157, 118, 79 and 40 columns and fails one under each; a grid left at the framework minimum still fits three columns in 22; `holds_in_counts_the_headed_summary_depth`; `a_grid_too_wide_for_its_window_draws_the_summary_alone` (three columns opened at 200 columns, drawn at 100: the summary's frame is the whole area, its title on the top row, no column divider and no command cell's text anywhere); `the_cells_stand_settled_in_the_first_frame_that_holds_them` (the same grid drawn again at 200: every cell in its settled rect, nothing in flight); `a_command_refused_in_a_small_window_opens_when_there_is_room` (first synced at 30 columns with two commands, then at 100); `a_hidden_grid_too_small_draws_nothing`. `render.rs` — `the_summary_alone_is_whole_marked_or_absent_at_every_width` (item 7).

README: the `[tiles]` section of `crates/cargo-tile/README.md` gains two sentences: command cells are never narrower than 40 cells, and a window too small for them shows the summary alone. Changelogs: Changed lines under `## [Unreleased]`.

**Files:**
- `crates/cargo-tile/src/wrap.rs` — boundary breaks, tests.
- `crates/cargo-tile/src/render.rs` — `draw_ancestry`, `ancestry_stem`, `ancestry_room`, `ancestry_rows`, `ancestry_lines`, `draw_summary`, tests.
- `crates/cargo-tile/src/constants.rs` — `WRAP_BREAK_AFTER`, `ANCESTRY_MIN_COMMAND_WIDTH`, `MIN_CELL_WIDTH`.
- `crates/cargo-tile/src/app.rs` — sets the grid's minimum cell width.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md` — the floor and the summary alone.
- `crates/tui_pane/src/bar/status_line.rs` — fit pass, test module.
- `crates/tui_pane/src/tiles/grid.rs` — `set_min_tile_width`, `holds_in`, tests.
- `crates/tui_pane/src/tiles/draw.rs` — the summary alone in `draw_tile_grid`, tests.
- `crates/tui_pane/CHANGELOG.md` — Changed lines.

**Seats:** 2 writers — the work splits by crate.
- `impl` — `crates/tui_pane/src/bar/status_line.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/CHANGELOG.md`.
- `test` — opens as impl: `crates/cargo-tile/src/wrap.rs`, `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md`.

**Constraints from prior phases:** Phase 1 added `draw_placements` under `draw_tile_grid` in `draw.rs`; the summary-alone branch goes in `draw_tile_grid` (`draw.rs:144`) after `grid.sync` and before `grid.drawing` and `draw_placements(buffer, area, &drawing, &demands, &widths, contents, cells)`, which stays the owner of the ordinary ground and frame. Phase 2 changed how `grid.rs` places moving pieces and column bands and added `TileGrid::drawing_at`; it changed nothing a settled grid shows. Phase 4 added `PathGroup::fitted_heading` and `HEADING_ELISION` in cargo-tile, `TITLE_ELISION` in `tui_pane/src/pane/constants.rs`, made the foot readout a list of whole candidates (`rows_readout_lines`), and made `text_default()` the terminal's own foreground on a transparent screen. `set_min_tile_width` is this phase's only new public item.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port`, `... lint tui_pane` and `... lint cargo-tile` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures beside a real `cargo check`, 50 rows: cells at 200, 126, 90, 64, 48 and 40 columns; the summary alone at 39, 32, 24 and 16 columns; and one window shrunk from 200 to 100 while three columns are open (the summary alone), then grown back to 200 (the cells back). Nothing is cut bare anywhere, ancestry breaks only at a boundary or carries `…`, the status line holds only whole items, and no command cell is drawn under 40 wide.
- A fresh helper's design check of all twelve shots: pass. The notice lists the shots on each side of the switch so the user can judge the width.

### Phase 6 — The summary takes the rows it asks for, not a second position  · status: todo

#### Work Order

**Goal:** The summary takes a further position in its column only when it would use more than half of it, so no rows stand blank in the summary while command cells are short of theirs.

**Spec:**

Evidence (Phase 2's design check, 200x50, the tenth command cell opening while the summary asks 12 content rows): the left column settles at 20, 13 and 12 rows where it was 16, 15 and 14; the summary shows seven blank rows; every other column holds four cells of 12 rows.

Cause as read; reproduce it in a failing test before changing anything. `summary_depth` (`grid.rs:1810`) deepens the summary as soon as `summary_share` (1789) at the current depth is under what it asks. `summary_share` calls `shares` (1702), which gives every piece its base share by weight (`apportion`, the summary's weight is its depth) and then serves the summary only from `spare`, the rows its column-mates do not ask for. In a 47-row column of four positions the base share is 11 or 12 rows and the mates want theirs, so a summary asking 13 is one or two rows short. It then takes a whole second position, 23 rows, and gives back only what its two remaining mates ask: 20, 13, 12, with seven rows nobody in that column wants, while the cell it pushed out crowds another column.

Rule:
1. Let `slot` be the summary column's base share for one position at the current depth (`apportion` of equal weights over the column's height). When the summary is short by `slot / 2` rows or fewer, it stays at this depth and takes the shortfall from its mates, the tallest first, one row at a time, never taking a mate under `min_tile_height`. Name the half as a constant in `crates/tui_pane/src/tiles/constants.rs`.
2. When it is short by more than that, or its mates cannot give the rows without going under the minimum, `summary_depth` deepens as today.
3. `summary_depth` stays a non-decreasing function of what the summary asks, with everything else fixed, so a summary growing a row at a time never flips between depths and back.
4. `queue_with_depth` and the motion code do not change: a depth change is still queued as Phase 2 left it.

Tests, all pure, on settled grids:
- `grid.rs`: `a_summary_one_row_short_takes_it_from_its_mates` — the evidence case: eleven positions, the summary at depth 1 with exactly the rows it asks, three mates sharing the rest, none under the minimum.
- `grid.rs`: `a_summary_far_short_still_takes_a_second_position` — a summary asking a slot and three quarters.
- `grid.rs`: `summary_depth_never_falls_as_the_summary_asks_for_more` — every demand from 1 row to the column's height, at 6, 9, 10 and 13 command cells.
- `grid.rs`: `no_mate_goes_under_the_minimum_to_feed_the_summary`.
- `draw.rs`: `the_summary_shows_no_blank_row_while_a_cell_in_its_column_is_short` — through `draw_tile_grid` with `StubCells` demands: when any cell in the summary's column has fewer body rows than it asks, the summary has no body row beyond what it asks.
- Existing tests that pin a depth: change only those the rule changes, and list each in the summary with its old and new expectation.

Changelogs: one Changed line each under `## [Unreleased]`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `shares`, `summary_share`, `summary_depth`, tests.
- `crates/tui_pane/src/tiles/constants.rs` — the half-position constant.
- `crates/tui_pane/src/tiles/draw.rs` — one render test.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — Changed line.

**Seats:** 1 writer + 1 tester — the rule is one hand in `grid.rs`; the render test is written from this Spec in `draw.rs`.
- `impl` — `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/constants.rs`, both changelogs.
- `test` — `crates/tui_pane/src/tiles/draw.rs`: the render test, failing until `impl` lands the rule.

**Constraints from prior phases:** Phase 2 made `TileGrid::drawing_at(area, growth, raw)` the pure entry to a motion and made `queue_with_depth` keep every surviving cell in a column it starts or ends the queue in; neither changes here. Phase 5 added `TileGrid::set_min_tile_width` and the crate-private `holds_in`, which counts `target_depth()`: a summary that stays shallower lets a grid hold in a smaller window, and `holds_in`'s tests must still pass. `cargo-handler` draws on the same grid, so its summary follows the same rule. No new public item; no `#[allow]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's capture at 200x50 with seven `+` and seven `-` (it reaches ten cells and four columns), transparent off: in every settled frame the summary shows no blank body row while a cell in its column is short of what it asks, and Phase 2's motion measures still read zero.
- A fresh helper's design check of the settled shots and one motion shot per step, against Phase 2's shots of the same steps: pass.

### Phase 7 — Slow tests start first  · status: todo

#### Work Order

**Goal:** In a full `cargo nextest run`, every test that takes 0.8 s or more on natedev or the Mac starts before any faster test. Ordering only: no test is rewritten (user's words: "for now").

**Spec:**

1. **Measure before.** On each machine run the suite CI runs, `cargo nextest run --all-features --workspace --exclude cargo-mend --tests`, three times with JUnit turned on from outside the repo config: a file `junit.toml` holding `[profile.default.junit]` / `path = "junit.xml"`, passed as `--tool-config-file tile-fixes:<path>/junit.toml`. Each `testcase` then carries its start `timestamp` and `time`. Record the wall time of each run and, per test, its slowest time. On the Mac work in a checkout of this branch outside `/tmp`, reached with `ssh mac`.
2. **The slow set** is every test at 0.8 s or more in any of the six runs, less the seven reader scenarios the config already orders. Known members to confirm by name with `cargo nextest list`: in `cargo-tile::unit_tests`, `progress::capture::…::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`, eight in `shim_registration::wire::`, two in `hook::`; two each in `cargo-tile::shim_modes` and `cargo-tile::cli_lifecycle`; in `cargo-handler`, `census::codex::…::reads_threads_by_id_whoever_started_them` and `…::reads_interactive_threads_created_in_the_span`; in `tui_pane`, `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw`.
3. **One override.** Append to `.config/nextest.toml` one `[[profile.default.overrides]]` with `priority = 80` and a `filter` that names each slow test exactly: `binary_id(<id>) & (test(=<name>) | …)` per binary, joined with `|`. No regular expressions, so a test added later is not swept in by accident. A comment above it says what the list is, the threshold, the date measured, and how to re-measure. nextest resolves each setting from the first override that sets it, so the two existing priorities (100, 90), the `cargo-tile-readers` group and both `slow-timeout` entries keep working unchanged; the new override sets `priority` alone and names none of the reader scenarios.
4. **Measure after**, the same three runs per machine.

**Files:**
- `.config/nextest.toml` — one override and its comment. In scope for this unit by the showrunner's word (2026-10-08), though the Units row does not list it.

**Seats:** 1 writer + 1 tester — one configuration owner and one measurement lane.
- `impl` — `.config/nextest.toml`.
- `test` — no file; runs the three before and three after measurements on natedev and the Mac, checks every exact filter name, and hands the measured slow set and start order to `impl`.

**Constraints from prior phases:** Phases 1 to 6 added render tests to `tui_pane` and `cargo-tile`; each runs well under a second and none joins the slow set.

**Acceptance gate:**
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, the source-switch reader is first, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.
