# tile-fixes

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-tile draws a moving cell on the painted ground, keeps every label and readout whole or marked, follows the system's light or dark setting, shows the summary alone when the window is too small for cells, sizes the summary to what it asks, and nextest starts the slow tests first.


> Renumbered 2026-10-12: the slow-test shortening was split out of the old phase 19 as phase 17; the old phases 17, 18, 19, 20 are now 18, 19, 20, 21.

> Renumbered 2026-10-13: phase 19's design-check follow-up was inserted as phase 20 (hard landing); the old phases 20 and 21 are now 21 and 22.

> Renumbered 2026-10-14: phase 20's follow-ups were inserted as phase 21 (hard landing); the old phases 21 and 22 are now 22 and 23.

> Renumbered 2026-10-16: the nextest-bar and CPU fixes were inserted as phase 21 and the ended-command trim as phase 22; the old phases 21, 22 and 23 are now 23, 24 and 25.

> Renumbered 2026-10-16 (second time): the showrunner moved the ended-command and empty-cell fix to run first after phase 20, then the hidden-commands fix; phases are numbered in run order. The old phases 22, 27, 21, 26, 23, 24 and 25 are now 21, 22, 23, 24, 25, 26 and 27.

> Renumbered 2026-10-16 (third time): the showrunner runs the hidden-commands fix right after phase 20, ahead of the empty-cell fix, because it shares files with phase 20's work; phases are numbered in run order. The old phases 21 and 22 are now 22 and 21.

> Renumbered 2026-10-21: phase 22 did not land; two phases were inserted after it, phase 23 (the user's argument and key items) and phase 24 (one lifecycle for every invocation, which carries phase 22's Work Order). The old phases 23, 24, 25, 26 and 27 are now 25, 26, 27, 28 and 29.

> Renumbered 2026-10-28: two phases were inserted after phase 28, phase 29 (move-only splits) and phase 30 (capture shim per-call cost). The old phases 29 to 35 are now 31 to 37.

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
- **Worktree:** `/home/natepiano/rust/cargo-liner-tile`, branch `remove-running-section-tile`. Every seat works only here. Set by the showrunner (production doc, Units table).
- **Stack:** Rust workspace, ratatui 0.30.2, cargo-nextest (0.9.143 on natedev, 0.9.136 on the Mac).
- **Layout:**
  - `crates/tui_pane/src/pane/` — `frame.rs` (shared borders, `draw_clipped`, titles), `chrome.rs` (tint and screen ground).
  - `crates/tui_pane/src/tiles/` — `draw/` (`paint.rs`, `measurements.rs`, `contents.rs`, `rows.rs`, `foot.rs`, `fade.rs`: the grid's drawing and the foot readout), `grid/` (`arrangement.rs`, `tile_grid.rs`, `bands.rs`, `motion.rs`, `allocation.rs`, `layout.rs`, `heights.rs`, `rows.rs`, `selection.rs`, `visual.rs`, `pieces.rs`, `demands.rs`: arrangement, motion, `fits`), `constants.rs`. Phase 29 split the old `draw.rs` and `grid.rs` into these modules. Each test file sits beside its owner as `<module>_tests.rs`; shared fixtures sit in `draw/tests/` and `grid/tests/`.
  - `crates/tui_pane/src/bar/status_line.rs` — the status line every app draws.
  - `crates/cargo-tile/src/` — `render.rs` (cells, headings, ancestry, status line notes), `wrap.rs`, `constants.rs`.
  - `.config/nextest.toml` — test ordering, groups and timeouts for the workspace.
- **Key files:**
  - `crates/tui_pane/src/pane/frame.rs` — `PaneFrame` (66), `draw_clipped` (196), `fill_pane` (229), `blit` (237), `GridLines::add` (436), `GridLines::add_titled` (480), `overlay_row` (599), `written_row` (642), `write_overlay` (661), `mark` (670), tests from 761 (`a_title_stops_inside_the_corner` 1071, `a_shifted_pane_keeps_only_what_lands_in_its_clip` 1132).
  - `crates/tui_pane/src/pane/chrome.rs` — `pane_fill` (129), `screen_ground` (145), `pane_background` (157).
  - `crates/tui_pane/src/tiles/draw/` — `paint.rs`: `draw_tile_grid` (60), `draw_placements` (96), `has_visible_body_row` (219), `draw_cell` (283), `draw_cell_with_rows_readout` (304). `rows.rs`: `draw_rows_readout` (209), `draw_rows_readout_after_foot` (229), `extent_readout_lines` (255), `draw_rows_readout_line` (331), `content_area` (347), `readout_row` (367, returns `ReadoutRow`). `foot.rs`: `draw_summary_foot` (38). Tests: `paint_tests.rs`, `rows_tests.rs` (`draw_tile_cell_draws_only_the_readout_on_its_foot_row` 328), `foot_tests.rs`, `fade_tests.rs` and `tests/motion.rs`; shared fixtures in `tests/mod.rs` (`StubCells` 67, `draw_motion_fixture` 272, `settled_grid` 555).
  - `crates/tui_pane/src/tiles/grid/` — `pieces.rs`: `TileDrawing` (119). `bands.rs`: `CellTransition` (22), `BandPieceMotion` (53), `moving_cell` (199), `wrapping_cell` (295), `column_bands` (338), `column_endpoint` (398), `place_band_pieces` (481), `band_piece_dividers` (753), `piece_frame` (1286). `tile_grid.rs`: `TileGrid` (138), `sync` (1170), `target` (1724), `queue_with_depth` (1761), `cycle_focus` (2323), `cell_at` (2392), `progress` (2520), `placements` (2533), `drawing` (2708), `drawing_at` (2792). `layout.rs`: test-only `Grid::new` (446). `allocation.rs`: `columns` (52), `shares` (145), `fits` (682), `summary_share` (690), `summary_depth` (720), `shared_run` (826). `motion.rs`: `lerp_rect` (43). Tests: `allocation_tests.rs`, `arrangement_tests.rs`, `tile_grid_tests.rs`, `tests/motion.rs`, `tests/motion_rules.rs` and `tests/summary_depth.rs`; shared fixtures in `tests/mod.rs`.
  - `crates/tui_pane/src/tiles/host.rs` — `cycle_step` (56), `hit_test_at` (92), `handle_tile_click` (104): the grid's keys and clicks.
  - `crates/tui_pane/src/app_config/theme_install.rs` — `install_theme` (28), `resolve_appearance` (44); `crates/tui_pane/src/theme/` — `state.rs` (`ThemeState`, `apply_system_appearance`, `set_active_theme`), `resolver.rs` (`AppearanceMode` 23, `resolve_active` 112), `poller.rs` (`spawn_appearance_poller` 185, `spawn_appearance_watcher` 197).
  - `crates/tui_pane/src/tiles/constants.rs` — `MIN_TILE_HEIGHT` (18), `MIN_TILE_WIDTH` (21), the `TILE_ROWS_*` readout constants.
  - `crates/tui_pane/src/bar/status_line.rs` — `render` (162), `status_line_note_spans` (205), `status_line_global_spans` (222), `render_sections` (272). No test module yet.
  - `crates/cargo-tile/src/render.rs` — `draw` layout (238), `Cells` and its `TileCells` impl (312), `summary_width` (438), `sccache_label` (716), `draw_ancestry` (939; truncation at 983), `ancestry_stem` (1130), `ancestry_room` (1144), `ancestry_rows` (1159), `ancestry_lines` (1180), `PathGroup::heading` (1373), `draw_path_group` (1634), `process_row` (1809), `heading_gauge` (2051), `cell_width` (2180), `draw_status_line` (2187), tests from 2247 (`buffer_line` 2421, `narrow_table_buffer` 4833, `buffer_rows` 5542).
  - `crates/tui_pane/src/wrap.rs` — the shared wrapper since Phase 7: `BoundaryWrap::push` and `wrapped`, which cargo-tile's command rows and ancestry call as `tui_pane::wrapped`; find each by name. cargo-tile has no `wrap.rs` of its own.
  - `crates/cargo-tile/src/constants.rs` — `ACCOUNT_HEADING_OPEN`/`CLOSE` (7, 9), `SUMMARY_CELL_TITLE` (159), `ELISION` (151).
  - `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — whole-frame test helpers `draw` (317), `draw_settled` (594).
  - `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/README.md` — user docs; each changelog has an `## [Unreleased]` section.
  - `.config/nextest.toml` — two priority overrides (100 and 90), group `cargo-tile-readers`, two `slow-timeout` overrides.
- **Test lanes:** `cargo-tile` — `crates/cargo-tile/tests/` (`unit_tests.rs` compiles every `src` module with its `#[cfg(test)]` tests into the binary `cargo-tile::unit_tests`). `tui_pane` — `crates/tui_pane/tests/`; frame, tile and status-line tests live in-module because they read private items. `cargo-handler` — in-module tests in the binary's own test target, among them `crates/cargo-handler/src/census/codex.rs`; find its exact binary ID with `cargo nextest list`.
- **Build:** `bash ~/.claude/scripts/direct/verify.sh check cargo-tile`
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test cargo-tile`
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`
- **Style:** clippy runs in each phase's `verify.sh lint`; the style review runs per phase
- **Invariants:**
  - A value on screen is whole, or shortened with a `…` that marks the cut, or absent. Nothing is cut without a mark (showrunner's gate for this unit: nothing clipped mid-value).
  - A command cell is never narrower than 40 cells in cargo-tile. A window that cannot hold the grid at that width shows the summary alone, filling the window (user, 2026-10-08: cells are useless when too small).
  - Every literal the code names lives in its crate's `constants.rs` (existing crate convention).
  - No test takes a second. Render tests are pure and run on buffers; none waits on the clock (user rule, memory `no-multi-second-tests`).
  - A `tui_pane` change reaches `cargo-handler` and `cargo-port`; a gate that touches `tui_pane` also runs `verify.sh test` for both (production doc, Merge tests). No existing `tui_pane` public signature changes, and `cargo-handler`'s cell sizes do not change (author's calls). The approved exceptions (the user, from the Phase 12 review) are the public signatures Phase 18 names — toast timing, toast action, toast focus and tracked-item timing — and `TileCells::group_title`, which Phase 36 changes. Phase 36's other public items are additions only: `draw_tile_grid` keeps its signature, and a retained boundary that answers `Option`, such as `cell_at`, converts from a named internal answer at that boundary alone.
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

#### As-built

- `pub(super) fn drawing_at(&self, area: Rect, growth: TileGrowth, raw: u32) -> TileDrawing<Id>` is the pure entry to a motion; `raw` is on the `PROGRESS_SCALE` scale before easing. `TileGrid::drawing` reads the clock once and delegates. `TileDrawing` keeps its name and its two fields.
- Column bands share their dividers. Each end's dividers are interpolated once and band `k` is the inclusive span `dk..=d(k+1)`: at every step the first divider is `area.left()`, the last is `area.right() - 1`, and each band's right divider is its neighbour's left. An opening or closing column collapses onto the last drawable column (test `column_bands_tile_the_area_through_a_transition`).
- Each cell of a step resolves to a `CellTransition::{PresentAtBothEnds { before_cell, after_cell }, Arriving { after_cell }, Departing { before_cell }}` and then to the ordered pieces of its band, `BandPieceMotion::{Resident { before, after }, Entering { edge, after, sliding }, Leaving { before, edge, sliding }}`. A band's divider rows are interpolated once and each piece is built from two consecutive dividers: a piece's bottom row is the next piece's top row, and the first and last boundaries are the band's frame rows (test `pieces_in_a_column_share_their_separators_through_a_transition`). A departure keeps its place in the band's order and closes onto the divider between its neighbours. Pieces keep transition order and are never sorted by interpolated rect (test `no_step_of_a_reorder_draws_two_cells_crossing`). Both types are crate-private.
- `shift_to_clip` shifts a sliding piece's border onto the divider its clip has, so the piece's first visible row is its own content and no band row is left on the screen ground (tests `no_line_is_doubled_through_a_transition`, `a_collapsing_cell_leaves_no_ground_only_row_between_neighbours`).
- `queue_with_depth` drops a queued step that would put a surviving cell in a column that is neither the one it has at the start of the queue nor the one it has at its end (test `a_queued_depth_step_keeps_every_cell_in_a_column_it_starts_or_ends_in`). A `sync` during an opening leaves the transition in flight untouched, and the claimed cell stays in its transition's column (test `a_cell_claimed_during_an_opening_stays_in_its_transition_column`).

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `drawing`, `drawing_at`, `CellTransition`, `BandPieceMotion`, `shift_to_clip`, `wrapping_cell`, `queue_with_depth`, column and row dividers, motion tests
- `crates/tui_pane/src/tiles/draw.rs` — `draw_placements` and `line_frame` draw the lattice and titles from the shared clip; render tests
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — Fixed entry

**Binds later work:**
- `drawing_at` is the pure entry to a motion; `drawing` reads the clock once and delegates. Motion tests drive `drawing_at` at 24 evenly spaced `raw` values per motion and build complete queued drawings.
- `BandPieceMotion` does not reach `draw_placements`: the renderer cannot tell a leaving piece from an entering one.
- `sliding: false` on a `Leaving` piece covers both an ordinary collapsing departure and a piece left in a closing column.
- `drawing_at` draws from the transition's start arrangement; `target()` is only the queue's end.
- Queue rule: at every queued step each surviving cell stands in the column it has at the start of the queue or the one it has at its end; `queue_with_depth` drops a step that breaks it.

**Gotchas:**
- A cell leaving a column the step removes is not sliding (`wrapping_cell`: `sliding: from_column < after.widths.len()`), so it is drawn as a whole shortened cell and its title shows twice.
- A settled cell with one body row draws its readout and no label.
- Steps queue one after another at 720 ms each; a key pressed during a motion waits for it.
- A capture with transparent on reads as unpainted by design; only transparent-off measures count.
- The pure render tests miss defects that a frame-by-frame replay of a live capture shows (`+`/`-` steps at 200x50, transparent off and on); a motion change is checked against both.

**Ruled out:** filling each band's interior with the pane ground in `draw_placements` after clipped content (the shifted border leaves no row to fill); changing `text_default()` to the terminal's foreground on a transparent screen (it breaks text on painted surfaces).

### Phase 3 — A closing column empties in one motion  · status: done

#### As-built

- A column that a step removes closes as one. `BandPieceMotion::ClosingWithColumn { before }` keeps each piece's starting rows and contents; the band keeps its starting top and bottom rows and only its left edge and width change, also where a widened summary shortens the band.
- `TileDrawing { pieces: Vec<TilePiece<Id>>, column_bands }`, `TilePiece { placement, name }` and `PieceName::{Shown, OnTheOtherPiece}` are `pub(super)`. `TilePlacement` and the public `placements()` are unchanged.
- `fn piece_name<Id>(index: usize, pieces: &[BandPiece<Id>], frames: &[PaneFrame]) -> PieceName` in `grid.rs` decides the name role from the placed frames. A cell with one piece is `Shown`. Of a crossing cell's two pieces the leaving one is `Shown` until the entering one's name row is visible (`draw::name_row_is_visible(frame: PaneFrame) -> bool`), or, when neither can show a number, until the entering clip is two rows tall.
- `draw_placements` draws a border title, a group title and an empty cell's number only on the `Shown` piece; the other piece draws frame, ground and readout.
- `CellAppearance<Id>` holds a cell's content and focus.
- Tests, `grid.rs`: `no_piece_in_a_closing_column_changes_height`. Tests, `draw.rs`: `a_title_is_on_screen_once_while_its_cell_moves_{right_and_columns_stay, right_and_a_column_closes, left_and_its_column_closes, left_and_columns_stay}`, `an_empty_cell_number_is_on_screen_once_while_it_moves_*` with the same four endings, and `a_departing_cell_keeps_its_title_while_its_border_is_visible`. One test per crossing arrangement keeps each well under a second.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — motion states, `TileDrawing`, `TilePiece`, `PieceName`, `piece_name`, closing-column test
- `crates/tui_pane/src/tiles/draw.rs` — `draw_placements` reads the name role, `name_row_is_visible`, name-once render tests
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — `## [Unreleased]` Fixed entry

**Binds later work:**
- The renderer reads `TileDrawing { pieces, column_bands }`; each `TilePiece` carries its `PieceName`, so the role and the placement cannot differ in count.
- `piece_name` is the one place that decides the name role. `draw.rs` never works it out from duplicate ids, and anything a cell shows once (border title, group title, empty cell's number) follows `PieceName::Shown`.
- `BandPieceMotion::ClosingWithColumn` stays a state of its own, separate from `Leaving`; a piece in a closing column never moves vertically or changes height.
- `no_piece_in_a_closing_column_changes_height`, the eight name-once render tests and `a_departing_cell_keeps_its_title_while_its_border_is_visible` keep passing through any change to `grid.rs` or `draw.rs`.

**Gotchas:**
- A cargo-tile job cell has no border title: its heading and ancestry are content, laid out by each piece separately, so a crossing job cell can show a row twice or not at all. `PieceName` does not cover it.
- A `-` pressed during a motion starts 0.64 to 0.93 s late; a capture frame at a fixed offset can predate the step, so a capture locates the step in the replay.
- The bare-row test helpers count a row as covered only by pane ground, a lattice glyph, or a title on a border row of the piece that draws it.

**Ruled out:**
- A title always on the entering piece: that piece starts one row tall, on a border row another title uses.
- A second vector of roles beside the placements: the two lengths could disagree.

### Phase 4 — `auto` follows the system's light or dark setting  · status: done

#### As-built

- With `appearance.mode = "auto"`, cargo-tile and cargo-handler read the system's light or dark setting before the first frame and follow a change while running, with no key press. A pinned `light` or `dark` ignores it. With no settings service the app starts dark at once.
- `tui_pane` remembers the last observed appearance as `RememberedAppearance::{NotObserved, Light, Dark}`, crate-private, an atomic in `ThemeState`. `resolve_appearance` reads it, so `install_theme` and `apply_settings` both resolve `auto` from it; `NotObserved` resolves dark.
- `tui_pane::apply_system_appearance(observed, &AppearanceConfig<I>) -> Option<String>` stores an observation, makes the theme it selects active and returns the missing-theme notice. One formatter, `theme::resolution_notice`, builds that notice; it names the theme in use.
- `tui_pane::spawn_appearance_watcher(on_change) -> Option<Appearance>` takes no config, blocks on the startup read, returns it, and calls `on_change` for later changes from its own named thread. Cost: one idle thread; on Linux one held session-bus subscription and no polling; elsewhere one `dark_light::detect` every 1500 ms (`POLL_INTERVAL`).
- Each app applies the startup observation before building its `App`, then drains later ones on the terminal thread (`drain_appearances` in cargo-tile) and redraws; the notice in `startup_note` follows the observed appearance.
- Tests: `applying_settings_uses_the_remembered_system_appearance` and `applying_settings_defaults_auto_to_dark_without_an_observation` at the settings site; `a_newly_observed_appearance_changes_the_active_theme`, `a_missing_light_theme_notice_names_the_selected_id` and `the_notice_follows_the_observed_appearance` in `tui_pane`; `applying_an_appearance_replaces_the_theme_notice` in each app; `the_summary_memory_total_reads_on_the_light_theme` in cargo-tile. None starts the watcher thread or reads the system.

**Files:**
- `crates/tui_pane/src/theme/state.rs`, `theme/poller.rs` — the remembered appearance, `apply_system_appearance` and its tests; `spawn_appearance_watcher`
- `crates/tui_pane/src/theme/resolver.rs`, `theme/mod.rs` — `resolution_notice`, the one notice formatter
- `crates/tui_pane/src/app_config/theme_install.rs`, `app_settings/step.rs` — `resolve_appearance` reads the remembered value; tests at the settings site
- `crates/cargo-tile/src/terminal.rs`, `app.rs`, `render.rs` — startup read, channel, `drain_appearances`, the light-theme memory total test
- `crates/cargo-handler/src/terminal.rs`, `app.rs`, `settings.rs` — the same wiring and its test
- `README.md` of cargo-tile and cargo-handler, `CHANGELOG.md` of all three crates — `auto` follows the system's light or dark setting; `## [Unreleased]` Fixed entry

**Binds later work:** Every writer of the remembered appearance runs on the app's terminal thread, and the watcher's callback only sends on a channel; the atomic depends on that. `theme/state.rs` holds four `#[expect(clippy::expect_used)]`, on its two lock reads and two lock writes, the number it had before; the user reviews every lint suppression, so later work adds none. No ink in the summary or the cells changed, so later work that changes one is checked on both themes, transparent on and off.

**Gotchas:** Tests that install process-wide theme state depend on nextest running each test in its own process. A system change between the startup read and the watcher's subscription leaves one frame stale until the next change. Contrast defects remain in palettes this phase does not edit: the status bar on both themes, pid inks on light grounds, green on the light summary ground.

**Ruled out:** A lock around the remembered appearance: two lint suppressions for no protection. Tests at the resolver alone: they stay green with the remembered value unread. Changing any ink: the memory total reads at 13.4:1 or better in all six captures.

### Phase 5 — A floor for cells, and the summary alone below it  · status: done

#### As-built

- **Cell floor.** `TileGrid::set_min_tile_width(&mut self, width: u16)` (public, `tui_pane`) writes `settings.min_tile_width`, never below the framework's `MIN_TILE_WIDTH` (8, unchanged; `cargo-handler` keeps it). cargo-tile calls it once where it builds its grid, with `MIN_CELL_WIDTH = 40`. `fits` refuses a cell that would make columns narrower, on `+`, on a command arriving and on `sync`: five columns need 196 terminal columns, four 157, three 118, two 79, one 40. A command refused a cell stays counted in the summary until `sync` opens it.
- **Summary alone.** `TileGrid::holds_in(&self, area: Rect, growth: TileGrowth) -> bool` (`pub(super)`) applies the `fits` test to every arrangement the grid may still draw: the one on screen, the start of the transition in flight, and each queued step, each at its own summary depth. When it fails, the crate-private `GridDisplay` is `SummaryAlone` and `TileGrid::drawing_at` returns one summary piece on the whole area through `summary_alone_drawing`, so drawing, hit-testing and focus read one answer. The summary's demand and measured width come from the whole area's inner width.
- **Keys and clicks while the summary is alone.** `cell_at` answers the summary for every point in the area; Tab's `cycle_focus` takes no step and reports the key unspent; arrow keys take no step and report nothing; the summary is drawn as the focused cell. The focus the grid holds is unchanged, so its cell has it again in the first frame that shows the cells.
- **No motion into or out of the state.** The grid keeps syncing and keeps its cells, which stand in their settled rects in the first frame whose area holds them. Hidden contents draw nothing; an area too small for a frame with one body row and one body cell draws nothing. `cargo-handler` gets the same rule at its 8-cell minimum.
- **Summary text.** In cargo-tile the summary is alone at inner widths up to 37 (area under 40). At every inner width from 6 to 37 each header, each table value and the foot is whole, shortened with `…`, or absent, through `elide_summary_end` and `elide_summary_start`; the end of a path survives.
- **Border title.** In `write_overlay`, an `OverlayStyle::Title` wider than its row draws its first `row.width - 1` cells and then `TITLE_ELISION`; a row one cell wide draws the mark alone. `OverlayStyle::Label` stays whole or absent.
- **Foot readout.** `readout_row(inner) -> ReadoutRow`, `ReadoutRow::{Reserved(Rect), CellTooSmall}`, is the whole last interior row inside the right inset; `content_area` reads only that. `rows_readout_lines(inner, rows, measured_at) -> Vec<Line<'static>>` lists whole candidates, widest first: `content rows: N[ @ W]  r/c: H/W`, then `content rows: N @ W` (only when `measured_at != inner.width`), then `content rows: N`. The first candidate no wider than the reserved row's room is drawn right-aligned, and nothing when none fits; no candidate is clamped. The row stays reserved either way, so no cell changes height. A cell with one body row gives it to its contents and draws no readout (`CellTooSmall`).
- **Status line.** Whole items only, grouped in the private `StatusLineItems` and fitted by `fitted_right_spans` and `fitted_left_spans`: the globals (` ? shortcuts`) while they fit with their one trailing cell; notes in front of them from the last note backwards; the left block only when it fits whole with one cell between, keeping the uptime and dropping navigation first; the centre only when it fits whole between them.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `set_min_tile_width`, `holds_in`, `GridDisplay`, `summary_alone_drawing`, `cell_at`, `cycle_focus`.
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/host.rs` — readout row and candidates; keys and clicks while the summary is alone.
- `crates/tui_pane/src/bar/status_line.rs` — whole-item fit and its test module.
- `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/pane/constants.rs` — marked border title, `TITLE_ELISION`.
- `crates/cargo-tile/src/render.rs`, `constants.rs`, `app.rs`, `shim_registration/app_scenarios.rs` — the summary at every width, `MIN_CELL_WIDTH` and `ELISION`, the floor call, two readout tests.
- `crates/cargo-handler/src/render.rs` — test-only golden for the status line (`OVERLAY_STATUS`).
- `crates/cargo-tile/README.md` (`[tiles]`), `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — the floor and the summary alone.

**Binds later work:** `TileGrid::set_min_tile_width` is public and is the only public item here; `holds_in` is crate-private; `drawing_at` is the one place that answers the summary alone, and `fits` is the one test that opens a cell. cargo-tile's `MIN_CELL_WIDTH = 40` is the width every command cell can count on. cargo-tile has one glyph constant, `ELISION` (`SUMMARY_TEXT_ELISION`, `TABLE_NO_COLUMNS_MARKER` and `ANCESTRY_ELISION` are gone), and directory headings and ancestry in command cells mark with it; `tui_pane` has `TITLE_ELISION`. `elide_summary_end` / `elide_summary_start` are the summary's shorteners. The toast and status line contrast work builds on `StatusLineItems` / `fitted_right_spans` / `fitted_left_spans`. `ReadoutRow::{Reserved, CellTooSmall}` decides whether a cell has a readout row, and no readout function returns a bare `Option<Rect>`. A command refused a cell stays counted in the summary until `sync` opens it; the cell height floor keeps that rule.

**Gotchas:**
- `holds_in` counts every position of each arrangement at its own depth (cells `+ TABLE_CELL + depth.saturating_sub(1)`), because `Grid::new` adds the summary's depth before it divides the columns; `fits` given the cell count alone accepts a deep-summary grid that does not fit.
- The transition's target alone is not enough: `drawing_at` draws from the transition's start, so a two-column target fits 79 columns while the three-column arrangement it closes from is still drawn under 40 a cell.
- Two test-only widths sit in cargo-tile's production `constants.rs`.

**Ruled out:** a `window too small` notice (no `TILE_GRID_TOO_SMALL` constant, no surface type); raising the framework `MIN_TILE_WIDTH`, since `cargo-handler`'s narrow cells are its own design; a second glyph constant in cargo-tile; an arrow-key "unspent" outcome; a draw-only summary branch in `draw_tile_grid`, which leaves `cell_at` and Tab working on a grid nobody sees; changing the toast action's `Option` type with the toast fix, a public API change reaching another crate.

### Phase 6 — Headings and ancestry are whole or marked  · status: done

#### As-built

- **Headings.** `PathGroup::fitted_heading` draws a directory heading whole or shortened with `…` at its head. It tries, in order: the whole heading; `[account] …/rest` (`component_heading`); `[account] …tail` (`tail_heading`, a tail of at least `HEADING_MIN_TAIL` = 8 cells); the same two without the account; the empty string. `heading_prefix` builds the account part.
- **Wrapping.** `Wrap::push` in cargo-tile's `wrap.rs` breaks an overlong word after any character of `WRAP_BREAK_AFTER = "/-=_.:,"`, else where it runs out.
- **Ancestry.** `mark_ancestry_cut` ends a block cut by its row budget in `…`: appended when a cell is free, else in place of the last cell. A level with fewer than `ANCESTRY_MIN_COMMAND_WIDTH` = 8 cells for its command draws its pid alone.
- **Table gaps.** `table_column_spacing(width, constraints, columns, command_width)` tightens the gaps between columns before a command wraps; `longest_command_width` and `command_line_width` measure the command.
- **Toasts.** cargo-tile and cargo-handler draw toasts inside their body rectangle, above the status line, by calling `Renderable::render(&mut app.framework.toasts, frame, body, &ToastsRenderCtx { now, pane_focus_state: PaneFocusState::Inactive })`. A card is as tall as its wrapped body (`drawn_line_count` and `drawn_input_line_count` in `toasts/body.rs`, plus an action row), and `render_body_elision` draws `…` at the end of its last visible row when it is cut.
- **The mark.** `TITLE_ELISION` is now `ELISION` in `pane/constants.rs`, with `pub(crate) const PaneFrame::ELISION` beside it.
- **Default Light.** The title ink is `Rgb(137, 86, 0)`, 4.51 to 1 on the status line's `Rgb(220, 220, 220)`. `status_line_text_meets_contrast_in_every_built_in_theme` holds every RGB ink to `MIN_STATUS_LINE_TEXT_CONTRAST` = 4.5.

**Files:**
- `crates/cargo-tile/src/render.rs` — headings, the ancestry mark, table gaps, the toast call
- `crates/cargo-tile/src/wrap.rs` — boundary breaks
- `crates/cargo-tile/src/constants.rs` — `HEADING_MIN_TAIL`, `WRAP_BREAK_AFTER`, `ANCESTRY_MIN_COMMAND_WIDTH`, the test-only contrast floor
- `crates/cargo-tile/src/theme/builtins.rs`, `crates/cargo-tile/themes/default_light.toml` — the title ink
- `crates/tui_pane/src/toasts/body.rs`, `crates/tui_pane/src/toasts/toast.rs` — the drawn line count and card height
- `crates/tui_pane/src/toasts/render/card.rs`, `crates/tui_pane/src/toasts/render/drawing.rs` — the cut mark and its tests
- `crates/tui_pane/src/overlays/frame_tail.rs`, `crates/tui_pane/src/overlays/mod.rs`, `crates/tui_pane/src/lib.rs` — no `render_toasts`
- `crates/tui_pane/src/pane/constants.rs`, `crates/tui_pane/src/pane/frame.rs` — `ELISION`
- `crates/cargo-handler/src/render.rs` — the toast call

**Binds later work:** The review-repairs phase starts from these names: `fitted_heading`, `tail_heading` and the test `a_directory_heading_keeps_the_most_informative_marked_tail`; `Wrap` and `WRAP_BREAK_AFTER`; `command_line_width`; `Toast::target_height(&ToastSettings)`, `drawn_line_count` and `drawn_input_line_count`; `PaneFrame::ELISION`; the two `Renderable::render` call sites; `status_line_text_meets_contrast_in_every_built_in_theme`. Public `tui_pane::render_toasts` does not exist. The cell phase's cut-row mark is the one `mark_ancestry_cut` draws, and its empty-column rule extends `table_column_spacing`.

**Gotchas:**
- A card's height is counted at the configured width before `render` has an area, so a card drawn narrower is cut and marked though rows are free.
- `Wrap::push` takes any boundary, so a flag's dashes can stand alone on a row at narrow widths (`--` / `workspac` / `e`).
- `tail_heading` caps the tail one cell short of the last component, so a heading that fits exactly loses a letter.
- The contrast test skips named colours; Default Dark's status line is unmeasured.
- The toast line count is a copy of ratatui 0.30.2's word wrapper and drifts if that wrapper changes.
- `elide_summary_start` exists only under `#[cfg(test)]`.

**Ruled out:** counting wide characters as two cells (the crate counts one cell per character throughout); renaming grid.rs's private `Grid`.

### Phase 7 — Review repairs: wrapping, headings and the toast  · status: done

#### As-built

- One boundary-aware wrapper lives in `tui_pane`: public `tui_pane::wrapped(spans, width) -> Text<'static>` collapses whitespace; crate-private `wrapped_preserving_whitespace` keeps whitespace runs for toast bodies, whose columns depend on padded labels. A boundary is taken after a character of `WRAP_BREAK_AFTER = "/-=_.:,"` only when the head it leaves fills at least half a line; a word on a line already holding text moves to a fresh line first, and a broken word's tail shares its continuation line with the words after it. Words carry a `WordSeparator { None, Indentation, BreakPoint }`. cargo-tile's own `wrap.rs` is gone.
- `command_line_width` measures program and arguments joined by one space, as drawn.
- `fitted_heading` tries the whole heading, `[account] …/rest`, `…/rest`, `[account] …tail`, `…tail`, then the empty string; `tail_heading` may keep the whole last component. `elide_summary_start` is gone.
- Ancestry: `ancestry_levels(ancestry, budget)` returns `AncestryLevel::AncestorAfterElision` for the nearest ancestor after a cut, so one row draws `… <nearest>` and two rows draw the elision row and the nearest; `ancestry_fit` keeps first, elision and last. `mark_ancestry_cut` marks a cut line.
- Toasts: `Toasts` stores `ToastDrawAreaWidth { Unconstrained, Drawn(u16) }`, set from the render area, and derives `card_width()` through `toast_card_width(settings, area_width)`; height, visible lines, entrance and exit timing, and a newly pushed toast's entrance all count at that width. Each explicit body line wraps on its own in its colour. The body has one cell of padding each side; when the area exceeds the card by fewer than `TOAST_FULL_WIDTH_SLACK = 8` cells the card takes the whole width. The card's clear rectangle is clamped to the area.
- `tui_pane::render_toasts(frame, framework, area)` is public again; cargo-tile and cargo-handler call it with `tui_pane::frame_inner(body)`, so toasts draw inside the tile frame.
- One `pub(crate) ELISION` in `crates/tui_pane/src/constants.rs`; `PaneFrame::ELISION` is gone.
- Default Dark's status bar is `Rgb(58, 58, 58)`; Default Light's `active_title` is `Rgb(137, 86, 0)`. `status_line_text_meets_contrast_in_every_built_in_theme` resolves named colours through an xterm table and checks four status inks on every built-in theme.

**Files:**
- `crates/tui_pane/src/wrap.rs` — the shared wrapper and its tests.
- `crates/tui_pane/src/constants.rs` — `ELISION`, `WRAP_BREAK_AFTER`, `TOAST_FULL_WIDTH_SLACK`.
- `crates/tui_pane/src/toasts/manager.rs` — drawn area width, card width.
- `crates/tui_pane/src/toasts/body.rs`, `toast.rs`, `commands.rs` — height and entrance at the drawn width.
- `crates/tui_pane/src/toasts/render/card.rs`, `drawing.rs` — padding, full width, clamped clear, buffer tests.
- `crates/tui_pane/src/overlays/frame_tail.rs`, `lib.rs` — `render_toasts`, `wrapped`.
- `crates/cargo-tile/src/render.rs` — callers, headings, ancestry levels.
- `crates/cargo-handler/src/render.rs` — the `render_toasts` call.
- `crates/cargo-tile/src/theme/builtins.rs`, `themes/default_dark.toml`, `themes/default_light.toml` — status colours and the contrast test.

**Binds later work:** toasts draw inside `frame_inner(body)` through `render_toasts`, and `toast_card_width` is the one place a card's width is decided (the toast gap changes it). `AncestryLevel::AncestorAfterElision` and `mark_ancestry_cut` are the ancestry cut mark that table marks reuse.

**Gotchas:** contrast is measured from the capture's cell colours, never from PNG pixels, where anti-aliasing blends the ink. The moved `#[expect(clippy::expect_used, reason = "tests should panic on unexpected values")]` lives in `crates/tui_pane/src/wrap.rs`.

**Ruled out:** counting wide characters as two cells (one cell per character); renaming `ToastDrawAreaWidth` (it stores the area width it names).

### Phase 8 — A cell shows a process or is not drawn  · status: done

#### As-built

- `pub fn TileGrid::set_min_tile_height(&mut self, height: u16)` writes `settings.min_tile_height`, never below the framework minimum. cargo-tile calls it beside `set_min_tile_width` with `MIN_CELL_HEIGHT = 6`: two frame rows, the column header, a directory heading, one process row and the foot readout. cargo-handler keeps the framework minimum.
- `shares` divides a column of touching pieces with `apportion_shared_run`, so `fits` and `shares` agree at the exact `shared_run` boundary; the last piece keeps the column's closing row, and no settled piece is drawn under the floor. `summary_share` judges framed height, and `band_piece_dividers` holds resident pieces at the floor through a motion. Entering and leaving pieces stay transition fragments.
- A command with no cell tall enough is refused through `fits` and counted in the summary until `sync` opens it, on Phase 5's refusal path.
- cargo-tile `render.rs`: `table_continuation` returns `TableContinuation { FullyDrawn, CommandWasCut, RowsWereOmitted }` per drawn path group, and `mark_table_continuation` places the `…`. A cut value carries it attached; a whole value followed by omitted rows carries it at the command column's far edge; an exactly fitting whole value first moves its last word to a free row below (`move_last_word_to_table_continuation_line`), and with no free row the mark takes its last cell. `mark_cut_line` is shared with ancestry. Column headers never carry the mark. A directory heading is drawn only with its first process row (`PathGroupDraw` carries `height_with_gap` and `continuation`). `visible_columns` drops a `compiler` column whose every row is `CompilerObservation::None` and a `runs` column whose every row reads 0; `Unknown` and `Unavailable` keep theirs.
- Toasts keep one cleared cell on each side (`TOAST_SIDE_GAP`) inside `frame_inner(body)`. `toast_card_width` subtracts both gaps, so wrapping, height and entrance timing count the drawn width. `join_adjacent_frame_columns` turns a covered frame tee into `│` only on a side whose gap touches the area edge.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — the height floor and shared-run apportioning, with their tests.
- `crates/tui_pane/src/toasts/manager.rs`, `toasts/render/layout.rs`, `toasts/render/card.rs`, `toasts/render/drawing.rs`, `crates/tui_pane/src/constants.rs` — the toast gap and frame joins.
- `crates/cargo-tile/src/render.rs` — table continuation marks, empty columns, heading admission, render tests.
- `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/constants.rs` — `MIN_CELL_HEIGHT` and its call.
- `crates/cargo-tile/README.md`, `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — the floor.

**Binds later work:** `TableContinuation` and `mark_table_continuation` are the table cut mark that a cut command filling its column, and a cut cell's trailing gap, extend. Under the framed rule a piece not at the bottom of its column may hold `min_tile_height - 1` raw rows; the summary's framed share builds on it.

**Gotchas:** a one-row header fragment in a capture can be a piece entering mid-transition rather than a settled cell; replay the capture's timing before calling it a defect. cargo-handler's snapshot expectations follow the shared-border rule.

### Phase 9 — A cut command fills its column, and a cut cell fills its rows  · status: done

#### As-built

- A command cut at its cell's foot fills the command column with hidden text up to one cell short of the column's width, then `…` (`cargo next…`, `nextest r…`). `TableContinuation::CommandWasCut { continuation: CutCommandContinuation }` carries the next wrapped line plus a `CommandContinuationJoin` (`Direct` for a punctuation split, `WordSeparator` for a space split, which rejoins with one space); `mark_table_continuation` appends it before marking. `RowsWereOmitted` and `FullyDrawn` keep their earlier behavior: a whole value before omitted rows marks at the far edge with its free cells, and an exactly fitting whole value wraps its last word onto a free row.
- `DrawnRow<'a>` keeps the wrapped cells in canonical column order, the line count and a borrowed `&CommandText`; `DrawnRow::cut_command_continuation(first_hidden_line)` builds the continuation only for the cut row, and `into_table_row` filters columns.
- `process_table_plan(groups, available_height) -> ProcessTablePlan { keep_gaps, visible_group_count }` measures every directory group before drawing. When all groups and gaps fit, every gap stays; otherwise every gap yields and groups are admitted in order while a heading plus first process row fits. Spare rows then sit once above the foot.
- A command refused for height stays in the summary until a resize gives it a cell.

**Files:**
- `crates/cargo-tile/src/render.rs` — `ProcessTablePlan`, `process_table_plan`, `CutCommandContinuation`, `CommandContinuationJoin`, `DrawnRow`, `TableContinuation`, and their buffer tests.
- `crates/cargo-tile/CHANGELOG.md` — Changed lines under `## [Unreleased]`.

**Binds later work:** `TableContinuation::CommandWasCut` carries a `CutCommandContinuation` built only for the cut row (`DrawnRow::cut_command_continuation`). A cut command's `…` sits at its column's far edge after filling the column with hidden text; any inset on the command column moves that edge. `ProcessTablePlan` (from `process_table_plan`) is planned before drawing: a table keeps every gap between directories when all fit, otherwise yields every gap, with spare rows once above the foot. A heading is admitted only with its first process row. Measurement of table rows goes through `DrawnRow`. Tests `a_table_that_yields_one_gap_yields_them_all_before_its_foot`, `a_fully_drawn_table_keeps_every_inter_group_gap`, `a_cut_command_fills_its_column_before_the_mark`, `a_cell_hiding_rows_leaves_no_blank_row_above_its_foot` pin this; `a_command_refused_for_height_remains_in_the_summary_until_it_opens` pins the summary listing.

**Gotchas:** Because a heading needs its first process row, one spare row under a whole command can remain when the next group needs two rows; this is accepted.

**Ruled out:** greedy earliest-first gap yielding, which drew uneven spacing; building continuation payloads per row per frame, which cost work on the height-only path; a bare `Option<T>` in place of the semantic `CommandWasCut` payload.

### Phase 10 — The summary takes the rows it asks for, not a second position  · status: done

#### As-built

- Half-position rule: `summary_depth` keeps the summary at its depth when `summary_share` leaves it short by at most half a position (`slot / SUMMARY_SLOT_SHORTFALL_DIVISOR`, `slot` from `apportion_shared_run` with the shared border restored). The shortfall comes from its mates, tallest first, one row at a time, never taking a mate's framed rectangle (after `share_borders`) under `min_tile_height`. Short by more, or mates unable to give, it deepens as before. `summary_depth` is non-decreasing in what the summary asks; `queue_with_depth` and the motion queue are unchanged. `cargo-handler` follows the same rule.
- `CellRowDemand { exact_framed, stepped_framed }` carries demand from `cell_wants` through `HeldCellLayout` to `shares`: depth and weights read `stepped_framed` (`TileSettings::demanded_rows`' three-row steps), spare-row placement reads `exact_framed`. A change in exact demand alone updates a settled layout next frame without queuing a motion; a shrinking cell's exact demand refreshes across a step boundary.
- Rule 5: within a column, no cell keeps a row past its exact framed demand while a column mate is below its own; such rows go to the short mates, shortest of demand first, never under `min_tile_height`. Only rows no piece asks for stand blank. Blank rows a cell draws inside its demand (gaps between directories) are content.
- Demand and drawn rows differ: a table given fewer rows than its demand yields every gap and admits a heading only with its first process row. `TileCells::rows_drawn(&self, content, area) -> u16` (provided, default `area.height`) answers the rows a cell draws in a given area; cargo-tile answers from `ProcessTablePlan::rows_drawn` (`content_rows_drawn`, `group_rows_drawn`, `process_table_rows_drawn`, `ancestry_rows_drawn`).
- `TileGrid::measure_row_steps` probes each cell/width/height once per frame into `CellRowMeasurements` (`ContentRowMeasurement`, `RowProbeLayout`). `shares_with_row_usage` moves rows a cell cannot use to the mate whose next content step they complete: `spare_rows`, `next_useful_step` (`UsefulRowStep`), `move_useful_step` (`UsefulStepTransfer`), recomputing spare rows after each move; a freed row no mate's next step fits stays put.
- `draw::content_area` is `pub(super)`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `shares`, `summary_share`, `summary_depth`, `shares_with_row_usage`, `measure_row_steps`, the measurement cache, tests.
- `crates/tui_pane/src/tiles/settings.rs` — exact and stepped demand.
- `crates/tui_pane/src/tiles/constants.rs` — `SUMMARY_SLOT_SHORTFALL_DIVISOR`.
- `crates/tui_pane/src/tiles/draw.rs` — `TileCells::rows_drawn`, the probe call, render tests.
- `crates/cargo-tile/src/render.rs` — rows-drawn answers and buffer tests.

**Binds later work:** allocation compares rows a cell draws (`rows_drawn`), never rows given; any view choice or frame pipeline serves that answer and the per-frame measurement cache.

**Gotchas:** blank rows remain only in motion (settled 0/3645 pieces, moving 195/569): content grows before height on a Together turn (~0.73 s) and eased heights land inside two-row content steps. Phase 12 (moving heights snap to heights content fills) owns them.

**Ruled out:** a content-side fix dropping gaps one at a time — it reverses the all-gaps-together look; the grid snap is the fix.

### Phase 11 — The summary shows alone when it shows more there, and a view setting picks the rule  · status: done

#### As-built

- `TileView { Auto, Summary, Cells }` (public, `#[default] Auto`, spelled `auto`, `summary`, `cells` in `config.toml`) is the `tiles.view` setting. It is framework-owned like `TileFill`: `FrameworkSetting::TileView`, `AppConfig::tile_view_mut`, and `SettingsRows::tile_view` push a `view` Stepper row. cargo-tile and cargo-handler store it in `[tiles] view`, push the row beside `fill`, write `view = "auto"` in their default config text, and call `TileGrid::set_view` each frame before `draw_tile_grid`.
- `TileGrid` holds one `held_display: GridDisplay`; `display()` returns it whenever the cells hold at their floor, else `SummaryAlone`, and drawing, hit-testing and focus movement all read it. `drawing_at` draws the summary alone when `held_display` is `SummaryAlone`, bypassing `GridMotion`. No view draws a cell under its floor.
- `draw_tile_grid` measures demands at both the cells widths and the alone widths, builds private `SummaryProbeDemands { beside_cells, alone }`, and passes it to `measure_row_steps(area, growth, summary_demands, rows_drawn)`. That pass measures the summary through `rows_drawn` in its settled cells piece and in the whole area, each capped at its demand at that width, then `commit_display` commits the view. The frame draws with the committed view's widths and demands.
- `commit_display` acts on settled frames only; a motion keeps the view it began with. `summary` is always alone, `cells` is today's floor rule. `auto` goes alone when the summary draws fewer rows beside cells than alone, and returns to cells only when beside cells it draws its whole ask or its piece has `AUTO_VIEW_RETURN_MARGIN` (3) rows to spare, so a one-row change in the ask never flips the view. `set_view` applies `summary` or `cells` at once on a settled grid; `auto` waits for the frame's probes.

**Files:**
- `crates/tui_pane/src/app_config/tile_view.rs` — `TileView`, its spelling and stepping.
- `crates/tui_pane/src/tiles/grid.rs` — `held_display`, `set_view`, `SummaryProbeDemands`, summary probes in `measure_row_steps`, `commit_display`, view tests.
- `crates/tui_pane/src/tiles/draw.rs` — demands at both widths, the committed view's widths, render tests.
- `crates/tui_pane/src/tiles/settings.rs`, `crates/tui_pane/src/tiles/constants.rs` — `TileSettings::view`, `AUTO_VIEW_RETURN_MARGIN`.
- `crates/tui_pane/src/app_settings/{step,rows,constants}.rs` — the `view` row.
- `crates/{cargo-tile,cargo-handler}/src/{config,settings,render}.rs` — `[tiles] view`, the row, the `set_view` call.

**Binds later work:** the held view is committed on settled frames and held through motion; moving heights apply to the cells view only. `measure_row_steps` takes `SummaryProbeDemands` and runs the summary probes beside the row-step probes. `draw_tile_grid` calls `cells.demands` twice per frame (cells widths and alone widths); any per-frame snapshot of demands serves both. Tests that pin cell layout select `TileView::Cells`. Tests `a_summary_hiding_rows_beside_cells_shows_alone`, `a_demand_change_in_one_frame_decides_the_view_from_that_frames_rows`, `a_one_row_change_in_the_summarys_ask_does_not_flip_the_view`, `the_view_holds_through_a_motion`, `summary_view_shows_the_summary_alone_while_cells_fit`, `cells_view_shows_cells_whenever_they_fit_at_the_floor`, `no_view_draws_a_cell_under_its_floor`, `the_view_row_steps_through_auto_summary_and_cells`, `the_view_row_shows_the_file_spelling` and each app's `summary_tile_view_round_trips` pin this.

**Gotchas:** under the default `auto`, a fixture with a tall summary beside cells collapses to the summary alone; render fixtures that assert cell layout must call `set_view(TileView::Cells)`.

### Phase 12 — A moving piece is drawn only at heights its content fills  · status: done

#### As-built

- In the cells view, each moving piece stands at a height its content fills or is whole at: `FrameHeightFit::Filled` or `Whole`, never `LeavesGap`, judged at `draw::content_area(frame_inner(rect))`. Settled layouts and the Phase 9 table plan are unchanged.
- `measure_row_steps(area, growth, progress, summary_demands, rows_drawn, rows_kept)` runs a moving frame in two passes on one clock. `measure_moving_height_rows` takes the interpolated drawing (`HeightSnap::Off`) and probes each `PieceHeightSnap::Search { start, end }` piece at its interpolated height, its previous height, one `MOVING_HEIGHT_PROBE_STEP` toward the end, the end, and the filled height of any gap answer; it then draws with `HeightSnap::Frame` and records the chosen heights in `MotionHeightPath`. `drawing_at` replays those heights at the recorded progress (`PieceHeightPath::DrawSelected`) and advances from them otherwise (`AdvanceFrom`).
- `snapped_band_piece_dividers` solves a band's shared dividers together. `piece_span_bounds` gives each piece an interval between its previous height and its destination (residents keep `min_tile_height`, `ClosingWithColumn` keeps its starting height, entering and leaving pieces have no floor); the intervals sum into cumulative divider bounds, and `nearest_band_piece_dividers` takes the candidate nearest the interpolated dividers, first with every piece filled (`ContentFitPolicy::Required`), then with fewest gaps (`Preferred`), else the interpolated dividers. `monotone_band_dividers` and `ordered_height_changes` keep each piece's height one-way; both ends equal the settled layouts.
- A motion starts from `LastSettledCellGeometry::Drawn`, the grid the last settled cells frame drew, when its area and growth match the grid's; otherwise the source is resolved again.
- `TileCells::rows_kept(&self, content, area) -> u16` (public, provided, default `rows_drawn`) answers the rows a cell retains before column mates may take the rest. Settled transfers (`spare_rows`, `move_useful_step`) read `rows_kept`; the motion snap reads `rows_drawn`. cargo-tile answers both from one measurement (`content_row_use`, `GroupRowUse`, cached per draw as `MeasuredCellRowUse`); a lone spare ancestry row is drawn but not kept (`ancestry_allocation`, `ancestry_row_use`), so a mate's next process step may take it.
- The per-frame cache is `FrameCellRowUsage` of `FrameContentRowUsage` (painted rows, `FrameHeightFit`, `RetainedRows`): one probe per cell, width and height per frame. Every settled frame measures again; there is no settled early return.
- `TileGrid::set_draw_layout` (`pub(super)`, called by `draw_tile_grid`): an area change mid-motion ends the step in flight at its destination, which draws inside the new area; returning to the source area does not resume it.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — the two-pass probe, the band divider solver, `MotionHeightPath`, `LastSettledCellGeometry`, the frame cache, `set_draw_layout`, tests.
- `crates/tui_pane/src/tiles/draw.rs` — `TileCells::rows_kept`, both answers passed to `measure_row_steps`, the `set_draw_layout` call, render tests.
- `crates/tui_pane/src/tiles/constants.rs` — `MOVING_HEIGHT_PROBE_STEP` and the test probe counts.
- `crates/cargo-tile/src/render.rs` — `rows_kept`, the shared measurement, the lone ancestry row, buffer tests.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — Fixed lines.

**Binds later work:** allocation reads two answers, `rows_drawn` for fit and `rows_kept` for donors, and any per-frame snapshot of row use serves both from one measurement and also serves the motion probes. The motion source is the recorded settled geometry. Tests `a_moving_piece_snaps_to_a_height_its_content_fills`, `a_snapped_height_moves_one_way_through_a_motion`, `a_column_still_tiles_its_band_while_pieces_snap`, `two_stepped_mates_both_stand_at_heights_they_fill`, `arriving_and_leaving_pieces_snap_without_a_floor`, `filled_motion_sources_begin_at_their_settled_heights`, `a_resize_during_motion_draws_every_column_inside_the_new_area`, `returning_to_a_motion_source_area_keeps_the_resized_destination`, `a_seven_cell_moving_frame_limits_row_probes`, `a_settled_frame_measures_each_cell_size_once_and_requests_no_frame`, `a_turn_that_grows_content_and_height_draws_no_blank_row`, `painted_and_retained_rows_share_one_measurement` and `a_summary_hiding_a_directory_leaves_no_blank_row_above_its_foot` pin this.

**Gotchas:**
- `queue_with_depth` queues a key's depth change as its own step, and each step's full re-division can hand rows to a mate short of exact demand and take them back on the next, so a height reverses across one key. Tests that call `collapse_queued_changes_for_test` never see it; when a unit-tested fix does not move the live result, instrument the live binary. "One key's change moves in one motion" owns the fix, with an entering piece of zero rows that draws its line at the column foot and a whole moving piece that keeps blank rows while a column mate cuts.
- cargo-tile buffer tests cannot drive fixed progress through `draw_tile_grid` (private clock); they pin reachable frames, and fixed-progress cases go through `drawing_at` in `tui_pane`.
- `SEVEN_CELL_MOVING_ROW_PROBE_COUNT` (18), `SEVEN_CELL_SETTLED_DRAWN_PROBE_COUNT` (8) and `SEVEN_CELL_SETTLED_KEPT_PROBE_COUNT` (7) are exact call counts; a change that adds a probe fails them.

**Ruled out:** a settled-frame early return cached on a key without content state — it served stale row answers after content changed; re-solving the motion source with current callbacks — the motion then starts from geometry that was never drawn; one `rows_drawn` answer for both fit and donors — painted rows and retained rows differ for optional ancestry.

### Phase 13 — One key's change moves in one motion  · status: done

#### As-built

- **One step per key.** A `+` or `-` key queues one step that carries its slot move and its summary depth together (`queue_with_depth`, `queue_steps`, `DestinationDepthStatus`); a depth-only step remains only for a key that moves no slot. `queue_with_depth` also serves `sync`: each intermediate slot arrangement takes its own `summary_depth(cell_wants(...))` and the final one the wanted depth. `queued_cells_keep_endpoint_columns` still filters the steps, and no timing constant changes (`TILE_ANIMATION_MILLIS` over the key's step count, floor `MIN_STEP_MILLIS`). Tests: `a_key_that_changes_slots_and_depth_queues_one_step`, `no_unchanged_cell_reverses_height_across_one_key`, `a_key_whose_new_widths_change_demands_still_moves_once`, `every_queued_state_keeps_surviving_cells_in_an_endpoint_column`.
- **Motion source.** A queued key starts from the heights the last frame drew (`last_drawn_motion_source`) when column bounds match, each cell has one unshifted piece and surviving cells meet their floor; otherwise from the resolved endpoint geometry.
- **Bounded lending.** In motion, a whole piece gives its spare rows to a band mate that is cutting content, but only the rows its endpoint heights permit, so no cell takes a row back within one key. The moving-height search probes `rows_kept` and a whole piece's compact height, and candidate scoring keeps the fit as a named state (`CandidateSpareState`, `SpareRowPreference`). Tests: `a_whole_moving_piece_gives_spare_rows_to_a_cutting_mate`, `a_cell_lends_no_row_it_must_take_back_within_one_key`.
- **Slide.** Arriving and leaving cells slide through intermediate heights: the search takes the nearest opening or closing height before it shares spare rows, and `monotone_band_dividers` rounds to the column total while keeping dividers one-way per column. Tests: `a_key_that_adds_or_removes_a_cell_slides_its_column`, `the_first_cell_below_a_summary_slides_in_and_out`, `the_last_cell_leaves_in_steps_with_its_narrowing_column`.
- **Line rule.** A piece adds its line, titled or not, only once it has a body row (`has_visible_body_row`); a bodiless arriving piece draws no edge row, and a departing piece keeps its line until its last row goes. Tests: `an_arriving_piece_draws_no_line_until_it_has_a_row`, `a_cell_arriving_at_a_column_foot_leaves_no_line_before_its_first_row`, `startup_cells_leave_no_line_before_their_first_row`.
- **Endpoint measurement.** The shared endpoint row measurement, which serves settled frames too, gives an unused content row beside a cutting column mate to a mate that can paint it (`has_content_gap_beside_cut`, `grows_content`, `frame_painted_rows`), through a second `Grid::with_rows_kept` pass in that case only, so a motion ends on the layout it rests on. `with_rows_kept`, `serve_exact_shortfalls` and `spare_rows` are unchanged. Test: `no_piece_rests_on_a_row_it_cannot_fill_while_a_mate_cuts_content`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — the step queue, the motion source, the moving-height search, the endpoint measurement and their tests; `DestinationDepthStatus`, `DemandSync`, `CandidateSpareState` and `SpareRowPreference` are private to its queue and candidate scoring.
- `crates/tui_pane/src/tiles/draw.rs` — the line rule for a bodiless piece and the render tests.
- `crates/tui_pane/src/tiles/constants.rs` — test values; `SEVEN_CELL_MOVING_ROW_PROBE_COUNT` is 29.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — the Fixed and Changed lines under `## [Unreleased]`.
- `docs/cargo-tile/as-built/summary-depth.md` — a key's depth change is part of that key's one motion.

**Binds later work:** The one-step queue (`queue_with_depth`, `queue_steps`, `DestinationDepthStatus`) is what any retargeting of a motion builds on. `last_drawn_motion_source` applies only when column bounds match, each cell has one unshifted piece and surviving cells meet their floor; every other case starts from the resolved endpoint geometry. Retained rows protect a donor; painted rows decide a receiving step. The endpoint second pass reads painted rows, so anything that feeds `rows_drawn` or `rows_kept` from a frame snapshot serves that pass from the same single measurement. A content change during a key's window (a project arriving in the summary) still draws blank rows until the depth steps, up to 0.49 s; it stays open, with `a_content_change_mid_motion_leaves_no_blank_row` as the test name for its repair. Every test named here keeps passing unchanged.

**Gotchas:** The retained-row answer (`rows_kept`) used as the receiving-step answer counts an optional ancestry row a command paints as no increase and leaves the summary one unusable row. `SEVEN_CELL_MOVING_ROW_PROBE_COUNT` (29) is the bound in `a_seven_cell_moving_frame_limits_row_probes` and covers the spare-row and compact-height probes. `collapse_queued_changes_for_test` collapses a key to one step and hides a reversal that spans steps, so the tests here drive the real queue through `TileGrid::apply(TileAction::Add)` and `apply(TileAction::Remove)`; after `apply` from `Settled`, `queue_steps` calls `advance` at once, so they advance by hand, never on the clock. The capture tools match pieces by position, so reversal and second-motion counts are upper bounds whenever a piece's content changes inside a key's window; the flags are valid only on keys with no content change. Speed evidence is each pair's median and the pooled figures; a p95 from a settled window under 30 frames is noise. Live motion probes need the controlled workload, where projects start and stop on the key.

**Ruled out:** A depth-only step queued beside the slot steps (rows the summary yields go to mates and come back within the key); one row answer for both donor and receiver (retained rows hide a row the receiver would paint).

### Phase 14 — One column grid, and every row clear of the right frame  · status: done

#### As-built

- **One column grid per frame.** Stacked command cells of one inner width draw the same columns at the same offsets, and a column has one width in every command cell that draws it. The layout runs in stages: `ProcessRowValues` (a row's cell texts, before any width) → `FrameCommandColumnGrid` (each column's intrinsic width, which optional slots are present and the longest command, fitted once over every row of every command cell in the frame) → `CellTableLayout` (that grid at one cell's inner width, which derives its own drops and spacing) → `LaidOutProcessRow` → `ProcessTablePlan`. The summary has its own `FrameSummaryColumnGrid`, fitted over the summary rows alone. Both wrap `FrameTableColumnGrid` and sit in `FrameColumnGrids`; `Cells` holds one for the frame, and its demands, drawing and row probes read it. The drawn and kept row answers stay one measurement, `FrameCellRowUse`. Test: `stacked_command_cells_share_one_column_grid`.
- **compiler and runs after command.** `command` is as wide as the frame's longest command line, capped by what the cell leaves; `compiler` and `runs` follow it after one gap, and slack stands after the last column. Test: `compiler_and_runs_follow_the_command_column`.
- **The hidden-rows mark.** The command column's width includes `TABLE_CONTINUATION_CLEARANCE` (one blank cell and the mark's cell) whenever a command cell in the frame holds more than one row, whether or not a row is hidden, so the mark's cell is reserved before any cut. A whole command's hidden-rows mark stands at the command column's far edge, the same place as a cut command's mark, with at least one blank cell after the command's last letter. Tests: `the_longest_command_keeps_room_for_the_hidden_rows_mark`, `a_whole_command_keeps_a_blank_cell_before_the_hidden_rows_mark`.
- **One cleared cell before the right frame.** Every content row of the summary and of every command cell leaves its last interior column blank: column headers, headings and their gauge, table rows, ancestry rows and cut marks, whose far edge is one cell in. Demands are measured at the same inset width (`CONTENT_RIGHT_INSET`, 1). Test: `no_content_row_touches_the_right_frame`.
- **Headers only over drawn values.** An optional column (`state`, `compiler`, `runs`) keeps its slot in the frame grid when any row in the frame holds a value; a cell draws that column's header and values only when one of its drawn rows holds one, and otherwise leaves the slot blank, so no position moves. A table is planned before its header is drawn: `ProcessTablePlan` records the rows that receive a visible line as `PlannedVisibleRows`, `rows_drawn` reads that plan, and `labels_column` answers for the cell. Slot presence and header visibility are separate fields. Test: `a_column_whose_only_value_is_hidden_draws_no_header`.
- **One drop rule.** The summary and command cells drop columns through one function, `columns_for_command_name`: while a command name (program plus its Short-view name) would wrap, the next column in `TABLE_COLUMN_DROP_ORDER` goes; arguments beyond the name still wrap down the column. Test: `columns_drop_before_a_command_name_wraps`.
- **Motion.** A content change during a key's motion leaves no blank row behind it: the motion takes the new content's destination on the same clock (`RetargetedMotionDestination`, `DestinationHeightBounds`), and the height search returns `FilledBandPath::Filled` or `FilledBandPath::NoFilledPath`, never a bare option. A piece that is entering or leaving slides. A cell that is neither entering nor leaving cuts no row while a mate in its column holds a spare row it could give, in moving and settled frames alike; a resident whose content shrinks gives its rows to a cutting mate at once, and an emptied resident stays at its height floor beside cutting mates. A command waiting for a cell claims a key's opening inside the running motion (`open_waiting_commands_in_current_motion`). `CandidateSpareState` reads `Neutral`, `FilledAllocation`, `WholeSpare` and `SpareBesideFilledAllocation`. Moving and settled frame medians are within 1 ms of the Phase 13 build. Tests: `a_content_change_mid_motion_leaves_no_blank_row`, `no_cell_cuts_a_row_while_a_mate_holds_a_spare_row_it_could_give`.

**Files:**
- `crates/cargo-tile/src/render.rs` — the staged column grids, the process table plan, the right inset, the drop rule, the hidden-rows mark and their tests.
- `crates/cargo-tile/src/constants.rs` — `CONTENT_RIGHT_INSET` and `TABLE_CONTINUATION_CLEARANCE`.
- `crates/tui_pane/src/tiles/grid.rs` — cell layout, motion and the frame's row budget: the content retarget and the filled-path search.
- `crates/tui_pane/src/tiles/draw.rs` — drawing of moving and settled pieces, and the render tests.
- `crates/tui_pane/src/tiles/constants.rs` — motion and floor constants, and test values.
- `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — the Changed line and the Fixed line under `## [Unreleased]`.

**Binds later work:** `FrameColumnGrids` is the one layout that demands, drawing, `rows_drawn`, `rows_kept` and the motion probes read, so a column changes in the frame grid, never in one cell. The mark's cell after `command` stays reserved. Motion cases left open for "A cell that enters, leaves or settles moves on the clock": a cell that opens while another slides in, and a cell admitted into a running motion, both appear at their end height, which is worse than the Phase 13 build (the live capture is to count 0 command cells appearing at once); and a staying cell is cut on a key that frees room. Summary cases left open for "One reading per command, one ancestry form, and a titled summary": the summary keeps blank rows between projects while cells hide a process row, and a child row stands above its parent.

**Gotchas:** A piece admitted into a running motion has no admission time of its own: `open_waiting_commands_in_current_motion` changes the slots, keeps the running transition's source and clock and seeds the new piece's previous height to 1, and `drawing_at` then draws the piece at the shared clock's position, not from its opening height. A fixture built from live heights did not reproduce the entry-at-height frame, so a motion repair is proved on the live capture. The capture renderer for shots loads one font, so U+2713 and wide characters draw as boxes in the PNGs while the terminal draws them.

**Ruled out:** Another repair of the end-height entries proved on a fixture alone (each one fixed its fixture and moved the defect to a neighbouring live state); that repair starts from the admission path instrumented on the live workload.

### Phase 15 — The toast reads in plain words, and a word that fits a line is never split  · status: done

#### As-built

- A toast body is a list of styled runs, each with its own line-break policy: `ToastRun` and `RunBreaks::{AtSpaces, Never}`, wrapped by `wrap::wrapped_runs`. A `RunBreaks::Never` run moves whole to the next line; one wider than the line is cut and marked with `…`. The policy is set by the caller building the `ToastRun` and lasts as long as the toast. The public `ToastBody` keeps its variants and methods, text bodies draw as before, and `ToastView` carries the body as the named `ToastRenderBody` in place of the `Option` `body_line_colors`.
- The first-run capture toast's body is plain words: no backticks, no shim mechanics, the toolchain name kept. Its undo command `cargo tile uninstall` is drawn in a highlight ink and stays whole on one line at 200, 64 and 40 columns.
- One wrapper serves toast bodies and cargo-tile's table cells (`process_row`). A word that does not fit the rest of a line moves whole to a fresh line when it fits there; only a word longer than a whole line breaks, after a hyphen if it has one, else at the width.
- A toast card keeps one cleared row above, below and between cards, and one cleared cell each side (`TOAST_SIDE_GAP`); the frame's border is untouched.

**Files:**
- `crates/tui_pane/src/wrap.rs` — the wrapper: public `wrapped`, crate-private `wrapped_runs` and `wrapped_preserving_whitespace`; tests `adjacent_spans_are_separate_words_joined_by_one_space` and `a_hyphenated_word_moves_whole_when_it_fits_a_fresh_line`.
- `crates/tui_pane/src/toasts/body.rs` — `ToastRun`, `RunBreaks`; test `a_nonbreaking_run_moves_whole_to_the_next_line`.
- `crates/tui_pane/src/toasts/render/` — the card's cleared margins, `TOAST_ROW_GAP` in `crates/tui_pane/src/constants.rs` beside `TOAST_SIDE_GAP`; test `a_toast_keeps_one_cleared_row_above_and_below` in `drawing.rs`.
- `crates/cargo-tile/src/capture.rs` — the capture toast's words; test `the_capture_toast_names_its_undo_command_on_one_line` at card widths 36 and 58. The title at line 56 still names the shim, and its test is at line 189.

**Binds later work:**
- `wrapped` treats every span boundary as a word boundary and joins words with one unstyled space; `wrapped_runs` keeps a word whole across a style change. cargo-tile's command cells depend on the first: `crates/cargo-tile/src/render.rs` ~3084 gives a program and its arguments as two spans with no whitespace between them, and joining them draws `cargocheck`.
- Any edit to `wrap.rs` runs cargo-tile's suite as well as `tui_pane`'s: only cargo-tile's tests fail when the span rule breaks (35 of 832).
- The hyphen break changes how many rows a long path takes in an ancestry level, so a test that counts wrapped ancestry rows measures with the wrapper, never by length.
- The frame check of the toast in motion at 64 and 200 columns, and of the workload that starts and stops projects on a key, has not run against this wrapper; the acceptance gate of "Readable inks, and a foot that fades" owns it, with its inputs and the seat that repairs a worse result.

**Gotchas:**
- The wrapper counts characters, not terminal cells (`cell_count`), so double-width text is measured short.
- `ancestry_fit` (`crates/cargo-tile/src/render.rs` ~1500) counts levels, not rows, and the mark takes a counted place: a chain with a multi-row level can leave a blank row above the foot while hiding a level it had room for.
- A hidden summary row sizes the frame's column grid, so a row nobody sees can drop `cpu` and `mem`.

**Ruled out:**
- Replacing the toast's U+2713 mark: Ghostty on the Mac is set to `font-family = Hack`, the installed Hack Nerd Font lacks the glyph, and CoreText falls back to Menlo, which holds it, so the terminal draws the mark. An image renderer that loads one font shows an empty box and needs a per-glyph fallback.
- Fixing the shim-naming title, the level-counting `ancestry_fit` and the hidden row that sizes the grid here: none comes from this wrapper or toast change, so each stays an open defect.

### Phase 16 — Readable inks, and a foot that fades  · status: done

#### As-built

- Every ink cargo-tile draws measures 4.5:1 or more on both grounds (`pane_background(true)` and `pane_background(false)`) in each built-in theme (Default Dark, Default Light, High Contrast, starter), transparent off and on; the lowest is 4.63:1. Contrast is measured from cell colours, never PNG pixels.
- Pid family colours are theme roles, eight `family_*` entries per theme. `family_color(index)` hands out only those clear of the reserved inks (label, error, default text, accent/heading, success) by one HSL hue-and-saturation clearance, `FAMILY_COLOR_CLEARANCE = 60.0` on a radius-100 saturation circle; every built-in theme yields at least five, pairwise separate.
- The over-budget count and the `@ W` width in the foot readout draw in the error ink; the summary's running arguments draw in the success ink.
- A fading cell's foot readout, and the summary's foot, blend toward the ground with the fade of the rows above them. `TileCells::readout_fade` (provided, default `CellReadoutFade::Unfaded`) returns `CellReadoutFade::{Unfaded, Fading(CellFadeProgress)}`; cargo-handler keeps the default.

**Files:**
- `crates/cargo-tile/src/theme/mod.rs` — family colour roles, clearance filter, contrast and clearance tests.
- `crates/cargo-tile/src/theme/builtins.rs`, `crates/cargo-tile/themes/*.toml` — per-theme inks and family colours.
- `crates/cargo-tile/src/constants.rs` — `FAMILY_COLOR_CLEARANCE`.
- `crates/cargo-tile/src/render.rs` — ink use and the cell and summary fade answers; `roster.rs` — test fixture.
- `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/*.toml` — shared theme roles.
- `crates/tui_pane/src/tiles/draw.rs` — `CellReadoutFade`, `CellFadeProgress`, the faded foot and readout inks; exported from `tiles/mod.rs` and `lib.rs`.

**Gotchas:** A per-channel colour distance cannot separate dark saturated inks from a dark grey label on a light ground; hue and saturation can. Any theme change must keep five family colours clear of the reserved inks. A pid takes its family colour only while its family's child rows are drawn.

**Ruled out:** a per-theme clearance or a branch on a theme's name — one mechanism serves every theme; one family set for both appearances — no set meets 4.5:1 on both dark and light grounds.

### Phase 17 — The named slow tests run in under a second  · status: done

#### As-built

Test-only changes; nothing a user sees changed, and every shortened test asserts what it asserted before (none split, weakened or ignored).
- `tui_pane` `tiles::grid::tests::unchanged_mates_stay_between_their_endpoints_during_plus_and_minus_motion` samples stable frames by integer height step and measures every asserted frame through `measure_row_steps`: 0.056 s, and an exhaustive 1001-frame comparison matched it. `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw` is shortened.
- `wire.rs` `observe_live_log` waits for cargo's stderr marker in the live log (200 ms bound, exit 98); `live_log_handshake_rejects_capture_buffered_until_cargo_exits` pins that the marker is seen live, not only at EOF. The `wire::darwin_birth_conversion_*` and `wire::non_json_arguments_preserve_both_quiet_spellings` tests are shortened.
- `reader_scenario.py` overlaps writer and reader startup, waits on a CPU readiness barrier, cleans up with SIGKILL, reads `/proc` instead of `ps`, and defers imports; `reader_scenarios.rs` signals readiness with a FIFO token and runs `python -S`.
- `parallel_cases.rs` is new; the `shared_capture::*` tests and the `shim_modes` tests (`extended_registration_is_world_readable_before_publication`, `unset_home_does_not_abort_either_capture_path`, `caller_descriptors_reach_captured_cargo`, `owner_repairs_parent_and_account_modes_without_touching_other_data`, `reader_accepts_live_writer_under_different_locale_and_timezone` at 0.572 s) are shortened.
- Two reader scenarios stay over a second: `reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` (1.70 s) and `reader_keeps_parent_family_across_its_only_childs_source_switch` (1.20 s), both at load 56.

**Files:**
- `crates/cargo-tile/src/shim_registration/{parallel_cases.rs, reader_scenario.py, reader_scenarios.rs, shared_capture.rs, wire.rs}`, `crates/cargo-tile/tests/shim_modes.rs` — shim registration test harness and its tests.
- `crates/tui_pane/src/attract/controller.rs`, `crates/tui_pane/src/tiles/grid.rs` — the two `tui_pane` tests.

**Binds later work:** the port of the unchanged-mates grid test keeps its step sampling and its per-frame measurement through the frame preparation. The two reader scenarios still over a second have a measured floor in the real reader's startup (release to first full frame ~0.51 s, to CPU rows ~0.44 s more, before the fixed 0.25 s observation); the slow-tests-first phase measures them and proposes a reader-startup speedup if they stay over a second on a quiet machine.

**Gotchas:** the remaining cost of the two slow reader scenarios is the production reader's startup, scan and first render (~0.96 s under load), not test scaffolding. The live-log handshake must observe cargo's marker live; an EOF-only tee fails the regression test.

**Ruled out:** further cuts to the reader tests' scaffolding — the cost sits in the production reader's startup.

### Phase 18 — Toast timing is a named state  · status: done

#### As-built

Toast timing, activation, focus and tracked-item lifetimes are named public states in `tui_pane`; no public field or getter holds a bare `Option` for them, and rendered output is unchanged.
- `ToastView::linger() -> ToastLingerState::{Live, Lingering { progress: f32 }}` and `ToastView::countdown() -> ToastCountdownState::{NoCountdown, CountingDown { remaining_secs: u64 }}` (`toasts/view.rs`).
- `ToastActivation<A>::{Informational, Opens(A)}` (`toasts/toast.rs`) holds a toast's action in `ToastSpec`, `Toast` and the push inputs in `commands.rs`; `body_rows()` states its row cost (0 or 1); `Toast::action()` returns `ToastActivation<&Ctx::ToastAction>`.
- `ToastFocus::{PaneUnfocused, PaneFocusedWithoutToast, PaneFocusedOn(ToastId)}` (`toasts/navigation.rs`) is computed per query from the pane's focus and the viewport. Focus queries take that focus as input: `Toasts::focused_id(pane)`, `focused_toast_id(pane)`, `dismiss_focused(pane)`, `handle_key_command(bind, pane)`, `handle_key(bind, pane)`, with `pane: PaneFocusState` supplied by `Framework::toast_pane_focus_state()`. `StackLayout` holds one `focus: ToastFocus`, which the card renderer reads.
- `TrackedItem.timing: TrackedItemTiming::{StartUnknown, Running { started_at }, CompletedWithoutStart { completed_at }, Completed { started_at, completed_at }}` (`toasts/item.rs`), with `is_completed()`; an unknown start stays unknown after completion.
- `TrackedItemView` carries `lifecycle: TrackedItemLifecycle::{Running, CompletedWithoutFade, CompletedFading { progress: f64 }}` and `elapsed: TrackedItemElapsed::{StartUnknown, Measured(Duration)}`, both produced by `Toast::view`.

**Files:**
- `crates/tui_pane/src/toasts/{view,toast,item,navigation,commands,slots,lifecycle,manager,running_tracker}.rs`, `render/{card,drawing,layout}.rs` — the named states, their producers and readers.
- `crates/tui_pane/src/framework/mod.rs`, `src/input.rs`, `src/lib.rs` — pane focus supply, key routing, exports.
- `crates/tui_pane/tests/toast_states.rs` — each focus state, unknown start, zero-linger completion, linger, countdown, activation.
- `crates/cargo-port/src/tui/app/{mod,dismiss}.rs`, `tui/app/async_tasks/*`, `tui/panes/actions.rs`; `crates/cargo-tile/src/render.rs`, `shim_registration/app_scenarios.rs` — callers on the new types.

**Binds later work:** any toast focus query or key handler needs the pane's `PaneFocusState`; none answers focus on its own.

**Gotchas:**
- `CompletedWithoutFade` is the zero-linger case and keeps the spinner plus frozen elapsed row, so output matches the earlier rendering.
- A focus getter that ignores pane focus reports a focused card in an unfocused pane; `leaving_the_toast_pane_suspends_selection_commands_until_focus_returns` pins it.

### Phase 19 — One reading per command, one ancestry form, and a titled summary  · status: done

#### As-built

Each command shows its own reading once, a cut ancestry has one inline form, and the Short view names a command by its words, its profile and one session runner.
- Readings: a directory heading draws no reading of a command that has its own cell, and no reading is drawn twice in one table. The `state` column stands only while a drawn row holds a reading.
- Ancestry: `AncestryLevel::{Ancestor, AncestorAfterElision}`; hidden levels are marked inline by `…` before the nearest kept level's pid, in that level's command ink. The fit counts rows, not levels: the root stays at a budget of two or more, the parent of the table's row is the last level kept, and a cut ancestry gives its gap row to one more level. `AncestryIndentation::{WholeChain, MarkedChain { inherited_mark_cells }}` keeps the root at one column in every chain and steps each level right of its parent, mark included. Short view draws one ancestry row with the pid whole: the nearest ancestor that is neither a shell (`TRANSPARENT_PROCESS_NAMES`, `nu` included) nor an exec wrapper (`EXEC_WRAPPER_PROCESS_NAMES`, matched on file name), and gives up the foot's pid link to the first row's parent when a shell sits between them. Long view keeps the whole chain and that link.
- Parent walk: `ancestor_pids` returns `AncestryWalk::{Ended(Vec<T>), LimitCut(Vec<T>)}` (`census/scan.rs`), carried by `TrackedGroup` to the drawing. A `LimitCut` chain opens with the inline mark in both views, and its cell answers `CellContentExtent::CutAtSource`, so its foot count draws in the cut colour; row counts and demands read measured rows only.
- Command text: `CommandText::named` / `command_name` keep the toolchain and subcommand words after leading flags and the values of `SUMMARY_HIDDEN_VALUED_FLAGS`, and end in ` …` exactly when words were dropped; Short view names a program by its file name, never its path, and no wrap row holds only `…`. `CommandText::line(SummaryDetail)` keeps `--release`, `--profile <name>` and `--profile=<name>` under both details. `CargoProfile::{Default, Release, Named(String)}` is read from cargo's own arguments up to `--`; a non-default profile draws as `[release] cargo build …` / `[name] cargo build …` in both views and survives every width cut.
- Columns: `SessionColumnWidths` sizes `cpu` and `mem` once per session for the machine (cores × 100%, total memory); a summed reading needing more cells widens its column once and it never narrows. The cut mark's cell after `command` is always reserved. `CellTableLayout::from_grid` drops `command`, then `state`, then `pid` when a table does not fit; `for_table` removes an empty `state` slot only for `TableKind::Summary`. Summary rows past the frame's inner height do not size the summary's grid.
- Rows: a cell's own command row is drawn first, and a parent stands before its child inside a group. The summary gives up its separator rows, earliest first, before a cell in its column hides a process row, and gives a spare row to a cell hiding an ancestry level. `summary_rows` promotes a hidden driver's child rows into the summary.
- Marks and titles: `SUMMARY_CELL_TITLE = " summary "` (`┌ summary ─`); the hidden-rows `…` takes the ancestry marks' neutral ink; the capture toast reads `Capturing cargo progress`.
- Family colour: `pid_style(row, layout, &AllocationFamilyLinks)` reads family links derived from each allocation's table plan, so a parent pid takes its family colour only while its child rows are drawn; the roster keeps palette identity apart.
- Headings: a heading drops its `[account]` tag before cutting its path, `…` marks only a cut that removes characters, and an unreadable directory with an account draws the label alone (`[natepiano]`).
- `tui_pane`: `CellContentExtent::{Measured, CutAtSource}` with provided `TileCells::content_extent` (default `Measured`), and `TileCells::readout_rows(content, area, measured_rows) -> usize` (default `measured_rows`), which leaves surrendered separator rows out so the foot count is red only when a process row is hidden. `CellReadoutFade` applies after the extent class.

**Files:**
- `crates/cargo-tile/src/render.rs` — summary and cell tables, ancestry, readings, marks, session column widths, family links, tests.
- `crates/cargo-tile/src/census/command_text.rs` — Short and Long command text, `CargoProfile`.
- `crates/cargo-tile/src/census/scan.rs` — `AncestryWalk`, the `nu` classification.
- `crates/cargo-tile/src/{roster,capture,terminal,constants}.rs`, `shim_registration/rows_readout.rs` — walk outcome, title, shell and wrapper lists, fixtures.
- `crates/tui_pane/src/tiles/{draw,mod}.rs`, `src/lib.rs` — `CellContentExtent`, `content_extent`, `readout_rows`, exports.

**Binds later work:**
- `summary_rows` copies subtree totals into promoted driver-child rows while the cell reads its own process, and a parent above one child repeats that child's readings; the reading-scope rule in `Each reading in one place, and every mark where its cut is` governs both.
- `CellTableLayout::for_table` removes the empty `state` slot only for `TableKind::Summary`, so a Long cell's empty slot still costs it `dur`/`start`.
- Ancestry work builds on `AncestryIndentation::{WholeChain, MarkedChain { inherited_mark_cells }}`.
- `CommandText::line(SummaryDetail)` keeps profile arguments; Short names drop them in favour of the `CargoProfile` tag.

**Gotchas:**
- Each repair to the ancestry indent moved the defect to a neighbouring chain form; tests pin the root column and each level's step for cut and whole chains in both views.
- A cut summary must drop an empty `state` slot before `pid`.
- Under 60–73 load average, tests that run under a second alone take 1.0–2.3 s.

**Ruled out:** repairing the summary/cell reading scope and the parent-repeats-child readings here — both predate this work and belong to the reading-scope phase.

### Phase 20 — Each reading in one place, and every mark where its cut is  · status: done

#### As-built

Each reading is drawn once beside its own row, summed from numeric contributions on the final table plan; every cut, hidden level and dropped tag is marked where it was cut.
- Readings: the census keeps each process's own use as `InvocationResourceUse { cpu: Measurement<f32>, memory: Measurement<u64>, compiler: CompilerObservation }` (`census/scan.rs`); the group lead is no longer overwritten with totals. A row reads its own process plus the descendants with no drawn row, formatted once from the final plan; a row whose children are not drawn (a cut, or the summary's one row per command) reads its whole subtree, so a pid reads the same in the summary and its cell whenever both draw the same contributors. A hidden child's compiler count joins its drawn ancestor before compiler columns are chosen. A sum with an unavailable contributor reads unavailable, and an unavailable reading draws a blank cell (`cpu_text` and siblings; measured zero draws `0%`); an unreadable memory total leaves no entry in the summary foot.
- Progress: a summary row's reading sits in its `state` cell, never in a directory heading; a command cell's heading keeps its own gauge. A nested cargo sharing an unverified driver capture leaves the reading in the driver's cell heading and on its summary row, once per invocation. `state` values are right-aligned on the `%` sign under a title ending on that column.
- Ancestry marks: `AncestryVisibility::{Whole, LevelsHidden, CutAtWalkLimit, WalkLimitAndHiddenLevels}` decides each Short mark: Short ancestry drops arguments, hidden lower levels end the row in ` …`, a cut basename ends in `…` right after its last drawn character, and a wholly hidden ancestry keeps the row's parent pid in the `parent` column. A Long level cut by its row budget fills its last row and puts `…` directly after the last character.
- Command column: Short names keep the subcommand whole and the argument mark spaced (` …`), wrap between words, and on the last visible row drop whole trailing words. Each frame picks one column set: optional columns leave before a subcommand is cut, argument tails yield before resource columns, `start` and `dur` leave together, `state` leaves before `parent`, and empty compiler and runs slots give their width to commands. An empty `state` slot keeps the frame's offsets with a blank header; only a table holding a reading titles it.
- Headings: when any readable heading in a table drops its account tag, every readable heading of that account drops it; other accounts keep theirs, an unreadable directory keeps `[account]`, and a dropped prefix before a whole segment reads `…/<segment>`.
- Rows and marks: a driver row carries the pid cargo dispatched and its child's `parent` names it; a parent pid with no row in its table draws in plain ink. The hidden-rows `…` sits at the row's right end in both views; tables reserve row-end cells (`TableRowEnd::Reserved`) only when their frame grid hides process rows. A column's spare rows restore summary separators before any stays blank above a cell's foot.
- Wrapping (`tui_pane/src/wrap.rs`): a word as wide as its column moves to a fresh line whole through `process_row`, and the latest permitted break wins, so a `/` after the last `-` takes the break.
- Toast: `Toasts::area_avoiding_frames(&self, area: Rect, frames: &[Rect]) -> Rect` (`tui_pane/src/toasts/render/drawing.rs`) keeps the card and its cleared gap outside frame feet, using room below short frames and the original area in other columns.
- Grid (`tui_pane/src/tiles/grid.rs`, `draw.rs`): one demand-measurement pass and one sync per frame, each cell measured at most once per height; `TileCells::prepare_frame` and `TileGrid::painted_summary_frames` (unreleased). Allocation uses painted rows for receivers and retained rows for donors through `DonorSupply`, `RecipientContentGain`, `ColumnTransfer` and `GridRowReadings`; every transfer increases (total retained use, total painted use) lexicographically, so the donor loop terminates by construction. A summary that borrows a row protects its retained content while its separators yield to a mate's hidden content.
- `TileView` is `{Summary, Cells}` (default `Cells`); a saved `auto` loads as `cells` through `#[serde(alias = "auto")]`.

**Files:**
- `crates/cargo-tile/src/render.rs` — table plan, per-row readings, `state` column, ancestry marks, command column fit, headings, row-end marks, toast area, tests.
- `crates/cargo-tile/src/census/{scan,process_identity,command_text,mod}.rs` — `InvocationResourceUse`, dispatched driver pid, command text; `scan_cpu_scenario_tests.rs`, `summary_totals_tests.rs`, `memory_tests.rs` — census tests.
- `crates/cargo-tile/src/{constants,roster,terminal,config,settings}.rs`, `shim_registration/{app_scenarios,rows_readout}.rs`, `reader_scenario.py` — literals, fixtures, the reader predicate.
- `crates/tui_pane/src/tiles/{grid,draw,constants}.rs` — allocation solver, one sync per frame.
- `crates/tui_pane/src/toasts/render/drawing.rs` — `area_avoiding_frames`.
- `crates/tui_pane/src/wrap.rs` — break points.
- `crates/tui_pane/src/app_config/tile_view.rs`, `app_settings/{rows,step}.rs`, `tiles/settings.rs`, `crates/cargo-handler/src/{config,render}.rs` — `TileView` without `Auto`.
- `crates/{cargo-tile,tui_pane,cargo-handler}/CHANGELOG.md`.

**Binds later work:**
- `commands.hidden_when_idle` is read throughout `render.rs` and `roster.rs`: it decides cells (`TrackedGroup::deserves_a_cell`), summary rows (`summary_rows`), driver-as-ancestry (`leads_as_ancestor`), cell order (`began`) and the attract screen's idle test (`draw`), and it threads through Cells and `FrameColumnGrids`; `Hidden commands are hidden always` filters before the grids.
- The donor supply's termination measure and the summary loan's retained-content protection hold in `Every cell moves toward one target layout`, which also designs without `TileView::Auto`.
- `CPU counts only what each process adds` builds on `InvocationResourceUse` and the blank unavailable cell.
- `Toasts::area_avoiding_frames` is public `tui_pane` API with cargo-tile its consumer; `prepare_frame` is unreleased.

**Gotchas:**
- `render.rs` grew by about 8,000 lines; find symbols by name, never by a line number written before this phase.
- An unavailable measurement draws blank, not `--`; `reader_keeps_parent_family_across_its_only_childs_source_switch`'s registration predicate also requires blank unavailable cells.
- `a_shared_state_slot_is_titled_in_each_table_that_keeps_it` asserts the table without a reading draws no `state` title, with every other title at the same offset.
- On a loaded Mac many render and reader tests cross one second; on natedev `startup_cells_leave_no_line_before_their_first_row` and `a_key_whose_new_widths_change_demands_still_moves_once` sit near 1.2 s.

**Ruled out:**
- The `--` placeholder for a missing reading — a blank cell stands.
- Surplus rows in a complete cell — placed in `No cell stands empty`, with its fixture.
- Four open findings — placed in `Phase 20's follow-ups: one table walk, plain caches, and whole marks`.

### Phase 21 — Hidden commands are hidden always  · status: done

#### As-built

A command whose subcommand is on `commands.hidden` gets no cell, no summary row, no share of any total and no column width, idle or busy; a listed command nested under an unlisted lead leaves with its whole subtree, and the lead and unrelated siblings keep their rows, readings and ancestry.
- Boundary: `Roster::visible(&mut self, hidden) -> VisibleRoster` (`roster.rs`), built once per frame in `render::draw`. It drops listed leads (`TrackedGroup::without_hidden_descendants` drops listed nested subtrees), re-sums each lead's readings through `scan::subtree_resource_totals` (now `pub(crate)`, the census's own cycle-safe accounting), assigns family colours and `parent_family` from visible rows only, and widens `SessionColumnWidths` from visible rows only. Every render reader (`draw`'s attract test, `Cells`, `summary_rows`, `memory_total`, `FrameColumnGrids::for_frame`) takes `&VisibleRoster`; no reader keeps its own test of the list, and the driver-as-ancestry role (`leads_as_ancestor`) is gone.
- Subcommand reading: `CommandText::subcommand` (`census/command_text.rs`) skips cargo global options, the separate values of `CARGO_GLOBAL_VALUED_OPTIONS = ["--color", "--config", "-Z", "-C"]` (`constants.rs`) and `+toolchain` selectors, so `cargo --offline port` is hidden; `CommandText::is_hidden` reads it, and `named_arguments` skips the same values.
- Names: config key `commands.hidden`, `DEFAULT_HIDDEN_COMMANDS`, `CommandList::Hidden`, settings row `hidden cargo commands`; a saved `hidden_when_idle` is read as a fallback.
- Retired keys: `tui_pane`'s `AppIdentity::RETIRED_CONFIG_KEYS` (provided associated const, default empty, `app_config/identity.rs`) lets `LoadedConfig`'s load and save (`app_config/loaded.rs`) drop declared retired TOML paths after merging. cargo-tile declares `Config::RETIRED_KEYS = [["commands", "hidden_when_idle"]]`, so the startup write moves the old key's values to `commands.hidden` and removes the old key; cargo-handler inherits the empty list.

**Files:**
- `crates/cargo-tile/src/roster.rs` — `VisibleRoster`, `Roster::visible`, `TrackedGroup::without_hidden_descendants`, visible-only family assignment, tests.
- `crates/cargo-tile/src/render.rs` — readers take `&VisibleRoster`; the list is no longer threaded through `Cells`.
- `crates/cargo-tile/src/census/command_text.rs`, `constants.rs` — subcommand reading, `is_hidden`, the valued-option table.
- `crates/cargo-tile/src/census/scan.rs` — `subtree_resource_totals` shared with the roster.
- `crates/cargo-tile/src/{config,settings}.rs`, `shim_registration/reader_scenarios.rs` — key, fallback, retired key, settings row, fixtures.
- `crates/tui_pane/src/app_config/{identity,loaded}.rs` — retired-key removal on write.
- `crates/cargo-tile/{README,CHANGELOG}.md`, `crates/tui_pane/CHANGELOG.md`.

**Binds later work:**
- Admission, withdrawal and leave/enter timing in `No cell stands empty` are decided on `VisibleRoster`; a group the user adds to `commands.hidden` mid-run is withdrawn on the next frame with no departure motion (`a_command_hidden_mid_run_leaves_on_the_next_frame`), and one taken off the list enters by the ordinary admission rule.
- `CPU counts only what each process adds` must change each row's own `resource_use`, since the roster re-sums leads from it through `subtree_resource_totals`; a bare `cargo-nextest nextest run` repair must keep `CommandText::subcommand` and `is_hidden` answering `nextest` for both spellings.
- `FrameColumnGrids::for_frame` takes `&VisibleRoster` (`Every cell moves toward one target layout`, `Phase 20's follow-ups: one table walk, plain caches, and whole marks`); carrying `ArgumentLayout` on `CommandText` in `Phase 20's follow-ups: one table walk, plain caches, and whole marks` keeps the global-option skip.

**Gotchas:**
- `Roster::observe` and `advance` still run on the raw roster, hidden groups included; only `visible()` filters, so anything deciding what reaches the grid reads the visible view.
- `Roster::visible` clones, filters and re-sums every group on each drawn frame (small at realistic group counts).
- The free `command_text::subcommand(argv)`, read by `is_excluded` (`commands.excluded`) and by `scan.rs`'s `observed_shim_match`, does not skip valued global options; only `CommandText::subcommand` does.

**Ruled out:**
- Option skipping for the excluded-list reader in this phase — not caused by this phase; placed in `No cell stands empty` as one option-skipping function shared by `CommandText::subcommand`, the excluded-list reader and the shim's skip list.

### Phase 22 — No cell stands empty: an ended command leaves at once, and a drawn cell shows its rows  · status: dropped (showrunner, 2026-11-06: not landed; the defects that remain live in Phases 31 and 32)

**Outcome (2026-10-21).** Two trees were measured with a paired live capture against main: round 18 (the item 14 model) and the round 16 tree. Both were worse than main on some counts. Round 18: never-drawn jobs 17, d80 time to census up to 6.01 s, new cells that start blank (F040) 7, and one lost live cell (F045). Round 16 tree: F040 13, lost live cells (F045) 10, census time worse than main, and CPU per drawn cell worse than main in d80 and d40. By the showrunner's ruling, phase 22 does not land, and its code is not on the branch. The round 16, 17 and 18 trees are kept as patches in the session folder, `phases/22/trees` (`/tmp/claude/delegate/03f9b80e-dbe5-47c6-b29e-a7e6587ab721/phases/22/trees`). The item 14 state model stays as the base for Phase 24, which also carries items 1 to 13 of this Work Order. Evidence: `shots39_reply.txt` (round 18) and `shots40_reply.txt` (round 16 tree) in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad`.

**Depends on:** Phase 20, Phase 21

#### Work Order

**Goal:** A command that ends while the grid's step queue holds a backlog leaves the grid on the next motion, so no cell stands empty behind the queue. From the user (2026-10-16, relayed by the showrunner): "i asked you to fix it so we wouldn't have cells that never have any text in them but i still see them - did that ever get implemented?" It was asked for as Phase 36 item 21 and is not built. The user reported it again later that day with a live shot under a busy build load (several `obsidian_knife` builds; `/tmp/claude-1000/-home-natepiano-rust-cargo-liner/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/images/21.png`, cargo-tile 0.2.78-dev) showing two cases: cells whose foot reads `content rows: 0 @ 51` that are still drawn and growing, with no process in them (case A), and cells whose foot reads `content rows: 5 @ 53` that draw a blank body (case B). Both must be gone. It runs first after Phase 20's checkpoint, ahead of every other todo phase (showrunner's ruling, 2026-10-16: it is a wrong result the user keeps seeing). Files and lines below are as of phase 20's checkpoint and may move; find each by name.

**Spec:**

1. **Cause.** The `tui_pane` grid steps through a queue of arrangements, 720 ms each (`TILE_ANIMATION_MILLIS`, `crates/tui_pane/src/tiles/constants.rs` ~199), up to `MAX_PENDING_STEPS` (64, ~214). `sync_demands` (`crates/tui_pane/src/tiles/grid.rs` ~2023) starts from `self.target()` (~2305), the last queued step, and closes the groups that ended only there, at the back of the queue. An ended command's cell waits, empty, behind the whole backlog.
2. **An ended command cannot sit in a queued step, by construction** (showrunner's type rule). On each sync the queued steps hold only live ids: for example, a trimmed step type whose slots can hold only ids present in the current demands, built when the demands arrive. No after-the-fact filter at drawing or advance.
3. **cargo-tile withdraws at once** (`crates/cargo-tile/src/roster.rs`). An ended command's group starts leaving the grid on the scan that sees it end; its text fade and its cell's shrink then run together on item 9's one timing, so there is no separate withdrawal at a shortened fade.
4. **Test, red first:** `a_command_that_ends_mid_backlog_leaves_the_grid_on_the_next_motion`: a grid with a backlog of queued steps, one of whose commands ends; the next motion closes its cell, and no frame after that motion draws it. Phase 36's motion rewrite keeps this test as a binding regression (its Constraints and gate say so).
5. **Case B: a drawn cell shows the rows it counts.** A cell whose foot counts content rows draws those rows; a cell with nothing to draw is not drawn (Phase 8's rule, a cell shows a process or is not drawn). The cause is not yet known. Diagnose it first from a live capture under build load like the user's, before any repair: read where the counted rows go (for example rows counted but hidden, rows of a group that ended, or rows drawn outside the body) and write the measured cause in the summary. The repair makes the blank body unrepresentable by type where the cause allows (the user's rule: no procedural guard for an invalid state). **Test, red first,** named for the behavior it pins (for example `a_drawn_cell_draws_every_row_it_counts`): it renders the cell and reads the drawn buffer back.
**The cell closes with its fade** (user, 2026-10-18: "tiles still close AFTER their text fades away instead of with their text fading away"). An ended cell's close runs on the same clock as its text fade: its height goes down from the first fade frame, and the cell and its text reach zero on the same terminal frame. A cell never stands at full height with faded text, and never closes after its fade has finished. Tolerance (showrunner, 2026-10-20): a close one or two alpha steps early after a single frame gap of up to 33 ms is not a break.

6. **A new command enters the grid only when it has rows to draw.** From the user (2026-10-18): "if a command finishes before it gets a cell - abandon trying to draw the cell". A waiting command that ends before it is placed never gets a cell, an opening or a fade; it leaves the waiting set on the frame its end is seen. From the user (2026-10-16, relayed by the showrunner): "i think that new cells appear to show no content rows and then to show content rows and that's just the life cycle but probably we shouldn't start adding them to the grid until we're ready to add them to the grid". Today a new command's cell opens and grows while its foot still reads `content rows: 0`, and its rows arrive later. Rule (showrunner's ruling, 2026-10-16): a cell enters the grid only once it has a content row to draw. Before that it gets no slot, no opening motion and no share of the column height, so its first frame already holds its content. A placed cell that drops back to zero content rows leaves the way an ended command does (items 1 to 3). By construction, per the user's type rule: a waiting command and a placed cell are separate types, and a placed cell is built only from at least one content row together with an allocation that draws it (its border, its foot and one content row: its first drawable allocation), so a placed cell with no drawable content cannot be represented; never a filter at drawing. Its opening starts at that first drawable allocation, never border-only, so its first visible frame holds a content row. A vacant cell opened by `+` is a separate, intentional state (it shows its cell number) and keeps its own opening. On departure a leaving cell owns its last drawable rows until its terminal frame: it is erased on the frame its allocation can no longer draw a content row, and its neighbours take those rows on that frame. Test, red first, named for the behavior (for example `a_new_command_joins_the_grid_with_its_first_row`): a command with no rows yet opens no cell and takes no column height; once its first row arrives, the cell opens and no frame draws it with zero content rows; a second test, `a_cell_that_loses_its_last_row_leaves_the_grid`, pins the drop back to zero; a third, `a_new_cells_first_visible_frame_draws_a_content_row`, reads the buffer on the cell's first visible frame and finds a content row there. Hidden is not a departure (Phase 21's rule, hidden always): a running group whose subcommand the user adds to `commands.hidden` in settings is withdrawn at once. `Roster::visible` drops it before every reader, so on the next frame it has no cell, no row and no share of any total, with no fade, no close motion and no frame that draws its cell empty; its neighbours take its rows by the ordinary resident motion. The departure rule above (owning its last drawable rows) applies to an ended command and to a cell that drops to zero content rows, never to a hidden one. A hidden group taken off the list mid-run enters by this item's admission rule. Test, red first: `a_command_hidden_mid_run_leaves_on_the_next_frame`, a running cell whose subcommand is added to the hidden list: the next frame draws no cell, row or total for it, and no frame after the edit draws its cell.
7. **A complete cell keeps no surplus height** (phase 20's F069, moved here). In c40x120 the last cell (heading `… 1879107 python3`, one source-cut ancestry level) draws three blank rows above its foot in a reduced fixture (one in the live capture): its demand, drawn rows and content all equal 5, yet the grid allocates 9. Measured cause (phase 20 round 24): `crates/tui_pane/src/tiles/grid.rs` ~3834 apportions the whole column height and ~3886–3911 keep the resulting shares except transfers to cells with content gains, so surplus stays in cells whose content is complete; `CellContentExtent` is not an allocation input (`draw.rs` ~393 passes it to the readout after allocation). Rule: the cap is allocation saturation, not source completeness. A cell is no taller than the smallest allocation, its borders and foot included, past which more height draws no further row, whether its content is whole or cut at its source: the archived fixture answers `CellContentExtent::CutAtSource`, yet more height reveals nothing, so a cap on source completeness would miss it. The readable floor takes precedence over the cap: a saturated cell below the floor stays at the floor. Surplus goes to a cell that can draw more, or stays as the column's free space. Likely the same cause as case B (item 5): test that first. The removed red test (`a_short_cell_with_hidden_ancestry_counts_only_drawn_rows`) is archived verbatim in `/tmp/claude/delegate/03f9b80e-dbe5-47c6-b29e-a7e6587ab721/f069_fixture_for_phase22.md`; it uses cargo-tile's private render helpers, so restore it as a cargo-tile integration test in `crates/cargo-tile/src/render.rs`, and add a separate `tui_pane` grid regression, `a_saturated_cell_keeps_no_surplus_height`, whose cell answers `CutAtSource` and draws no further row with more height. Phase 20's allocation solver (F054: `DonorSupply`, `RecipientContentGain`, `ColumnTransfer`, `GridRowReadings` in `grid.rs`) moves rows only as donor supply that keeps protected content, with a termination measure; the repair keeps that guarantee and its two regression tests (`a_donor_whose_row_use_jumps_as_it_shrinks_keeps_its_used_content`, `a_startup_column_with_nonmonotone_row_use_finishes_sharing`).
8. **Case A is checked live, not assumed.** Items 1 to 3 answer the backlog cause and item 6 the new-cell cause. If the live capture shows a cell with `content rows: 0` still drawn and growing for any other cause (a fade, a group kept for its settings, a demand never withdrawn), name that cause in the summary and repair it in this phase.
9. **A cell leaves as its text fades, in one motion.** From the user (2026-10-16, relayed by the showrunner): "i think that tiles should start disappearing over the time frame that the text fades away - which can clip some of the text obviously if the cell is tall.  but that's fine with me.  i think it would look smoother that way rather than text disappearing and then the empty cell disappearing - tile fix can do that". Showrunner's calls: a leaving cell starts shrinking when its text starts fading, with the same duration and easing for both, and its neighbours take its space over that same time; clipping the text inside the shrinking cell is fine; there is never a frame with an empty cell. Entering mirrors it: a cell grows while its text fades in, over the same duration, and still enters only once it has a content row (item 6). This replaces item 3's separate withdrawal at a shortened fade. One timing authority, a cell's lifecycle timing (for example `CellLifecycleTiming { duration, easing }`), drives both its text alpha and its geometry, so the two cannot drift. Leaving takes the configured fade (`TilesConfig::fade()`, `crates/cargo-tile/src/config.rs` ~126: `DEFAULT_FADE_SECONDS` 3 s, up to `MAX_FADE_SECONDS` 30 s), the span the user named; entering takes the 720 ms opening (`TILE_ANIMATION_MILLIS`), with the text fading in over the same 720 ms. Resident motion keeps its 720 ms step. Tests drive the test-only motion policy with injected time; no test runs the animation at real speed. `a_leaving_cell_shrinks_on_its_text_fade` runs at a 3 s and a 30 s configured fade, and `an_entering_cell_grows_on_its_text_fade_in` at the 720 ms entry; each reads text alpha and cell height from the same frames.
10. **Order.** Hold `grid.rs` early under this phase's reservation, so the trim lands before handler-enh's overflow phases 2–3 edit `grid.rs` and `draw.rs`.
11. **An excluded command stays excluded when cargo options come before it** (showrunner's ruling, 2026-10-17: current work, wrong results). Phase 21 taught `CommandText::subcommand` (`crates/cargo-tile/src/census/command_text.rs` ~116) to skip cargo's global options, their separate values (`CARGO_GLOBAL_VALUED_OPTIONS`, `crates/cargo-tile/src/constants.rs` ~531) and `+toolchain` selectors, so `commands.hidden` matches `cargo --offline port`. The free `subcommand(argv)` (~488), which `is_excluded` (~506, `commands.excluded`) and `census/scan.rs` `observed_shim_match` (~2668) read, skips only flags, so `cargo --config k=v berth claim` takes `k=v` as its subcommand and is not excluded. Rule (with item 12): one table, `CARGO_GLOBAL_VALUED_OPTIONS`, and one option-skipping function serve all three call sites (`CommandText::subcommand`, the excluded-list argv `subcommand`, the shim pass-through); no second table and no scattered match. The two Rust readers share that function. Test, red first: `an_excluded_command_after_a_valued_global_option_is_excluded` (`cargo --config k=v <excluded>` is excluded), in `command_text.rs`; phase 21's hidden-path tests there pass unchanged.
12. **A workspace UI passes straight through the shim when cargo options come before it** (showrunner's ruling, 2026-10-17). The capture shim (`crates/cargo-tile/src/cargo-capture-shim.sh`, the `first=` loop and `case $first` ~73–103) reads the first argument past `+toolchain` only, so `cargo --offline port` runs through the tee instead of passing straight through (and `cargo --offline berth` is captured). A POSIX shell cannot read a Rust const, so the installed shim's skip list is rendered from `CARGO_GLOBAL_VALUED_OPTIONS` (for example a placeholder in `cargo-capture-shim.sh` filled where `crates/cargo-tile/src/hook.rs` builds the shim it writes and compares: `SHIM_SOURCE` ~75, `install` ~747, `write_shim` ~797), and the shim skips flags and those options' values before reading the subcommand; `cargo --version` and `cargo -V` still capture nothing. Tests, red first: `cargo_offline_port_passes_straight_through_the_shim` (a staged shim run of `cargo --offline port` reaches the real cargo without a capture), beside the shim tests in `crates/cargo-tile/src/shim_registration/shared_capture.rs`; `the_installed_shim_skips_every_valued_global_option` (the rendered shim names each entry of `CARGO_GLOBAL_VALUED_OPTIONS`), in `hook.rs`.
13. **A viewport too small for every cell gives whole cells only** (showrunner's ruling, 2026-10-18). A drawn live cell keeps its cell, its pid row and its foot on every frame, with one exception: when the viewport is below the required minima, from a shrink or from a growth of the minima, live cells can move into the summary count (showrunner, round 11). The shared frame check computes that pressure itself. The capacity outcome is the only source for paint, hit testing and keyboard focus: each frame, every required command is either a whole drawn cell or in the summary set, and the type computes the summary set as the required commands minus the drawn ones. They never become cut cells. Ended cells drop first. Then whole live cells leave, newest first, and the oldest stay in place. When space comes back, the cells in the summary count return as whole cells, oldest first. By type: the divider bounds cannot hold a minimum above a maximum (their constructor returns the capacity failure, so no clamp can panic), and the drawing returns only rectangles inside the viewport, from the same outcome. The shared frame check holds this rule. Tests: a shrink below the required minima, and a resize back to full size; the shared check passes on every frame of both.

14. **Invocation state model (round 18; showrunner ruling 2026-10-20: written before code, approved with four additions).** One command invocation, from discovery to the end of its cell, has one owner record and one state. Today the state is spread over five places (`ProcessPresence`, `RetainedCommandProcess`, `RowLifetime`, `CellDeparture`, `CellLifecycle`), "ended" is only inferred from absence (`roster.rs` ~497), and `Missing` reads as `Live` for admission (`render.rs` ~419, `roster.rs` ~204). Round 17's live capture shows the result: cells admitted after their command's real end (rule e), fade-ins that run past the end (F041), rows that join after the end (F044), jobs held out of the census for up to 3.9 s, and one live child in two groups (F030).
   - **The record.** `census/scan.rs` keeps one map `Invocations: HashMap<InvocationId, InvocationRecord>`. It replaces `ProcessTreeContinuity.nodes`, `BranchingLifetime` retention and the `Scan.missing` list. `InvocationRecord { owner: InvocationOwner, membership: Membership, state: InvocationState }`. Only the census module can construct a record. The five owner construction sites stay the only ones: first observation (native or captured) and replacement after a contradictory lifetime.
   - **States** (`enum InvocationState`):
     - *Unseen* is not a value: an invocation without a record. A record is created only from a discovered process with argv or a confirmed capture registration, and it starts `Live`.
     - `Live { rows }`: seen in this scan. `rows` is the invocation's current row list.
     - `Missing { rows, since: ScanNumber }`: not in this scan's discovery, and the kernel says the same process still exists: `kill(pid, 0)` succeeds, the process is not a zombie, and its lifetime does not contradict the record. A probe that cannot answer (EPERM) also gives `Missing`. `Missing` keeps the rows of its last `Live` scan and adds none. A `Missing` record becomes `Live` again only when discovery sees it with the same lifetime; a different lifetime is a new invocation with a new owner.
     - `Ended(EndedInvocation { rows, at: Instant })`: the end is proven. Proof is any of: the probe returns ESRCH, `/proc/<pid>/stat` reads zombie (`Z`) or not found, a confirmed registration reads `Ended`, or a contradictory lifetime shows the pid was reused. `rows` is frozen from the last `Live` or `Missing` state. `Ended` is terminal: it has no method that adds a row, changes a row or returns to `Live`. A process that comes back under the same pid with a new lifetime is a new invocation with a new owner.
   - **When `Missing` becomes `Ended`.** The probe runs for every `Live` or `Missing` record whose pid is absent from discovery, in the same scan, before publication. A process that really ended is therefore `Ended` on the first scan that does not see it (a zombie is ended). A process that is gone but cannot be probed stays `Missing` for at most `MISSING_SCAN_LIMIT` scans (2); after that the record becomes `Ended` with proof `NotSeen`. `Missing` can never last without a bound.
   - **Record removal.** An `Ended` record leaves the map when its cell's departure finishes, or on the scan after it is published when it never had a cell. After removal only the lifetime facts that pid-reuse detection needs are kept.
   - **Capture-only invocations.** A record built from a confirmed registration (empty argv, no tree node) is a normal record. When its registration reads `Unknown`, it is `Missing` (it is not dropped); `Ended` or not found on the registration makes it `Ended`. Readable native metadata for the same process joins this record (the captured owner is kept); it cannot make a second owner.
   - **Group membership.** `enum Membership { Lead, Member { lead: InvocationId } }` lives in the record, so one invocation is in exactly one group by construction; groups are built from memberships, not walked again from the process tree each scan. A child keeps its `Member` link while its lead is `Live` or `Missing`. When the lead becomes `Ended`, each child that is still `Live` moves on that scan to the nearest `Live` or `Missing` ancestor, or becomes `Lead`. The ended lead's frozen rows then hold only the rows that ended with it. A `Missing` parent can no longer let its live child appear as a second group. What the user sees when a lead ends with a live child: the child, now a `Lead`, gets its own cell through normal admission (it is `Live`), and the ended lead's cell leaves with only its own rows. Nothing moves rows between drawn cells within one frame.
   - **Publication.** Each scan publishes every record once: `Live` and `Missing` as live invocations, `Ended` as an `EndedInvocation` value (the end is a value, not an absence) on the scan that proves it. No known invocation is held back: `IncompleteInvocationFamilies` and its `retain_ready` go away, and a row with incomplete details is published with the fields it has. Metadata reads never block each other: a fixed pool of 4 reader threads takes requests from one queue, a read that is still running at the scan's deadline keeps its thread but frees its request (retried on the next scan), and when every thread is stuck the scan still publishes on time with the fields it has; no scan waits. The other synchronous steps on the publish path (the full process refresh, `Capture::take`, attribution) are not changed in round 18; they go to Phase 24 with their measured cost.
   - **Roster.** `RowLifetime` is derived from the published state and cannot disagree with it: `Live` and `Missing` give a live row, and only an `EndedInvocation` gives `Finished(at)`. Absence from a scan never finishes a row. A finished row cannot be refreshed: `TrackedGroup::refresh` takes only live invocations.
   - **Drawing and admission.** Admission takes an `AdmissibleInvocation` that only a `Live` invocation can give (not `Missing` and not `Ended`), so an ended command cannot get a new cell and a missing one cannot get a first cell. A drawn cell of a `Missing` invocation stays drawn as live. `CellLifecycle::Leaving(EndedCell)` is built only from an `EndedInvocation`. When the end comes during the entry fade, the departure starts from the ink drawn on that frame and the cell's rows freeze at once. An ended cell paints only its frozen rows.
   - **Tests, red first** (each runs real census continuity, the roster and rendered buffers, under a second): `a_process_that_exits_ends_on_the_first_scan_that_misses_it` (zombie included); `a_command_that_ends_while_waiting_never_gets_a_cell`; `an_end_during_the_entry_fade_freezes_its_rows_and_ink`; `a_missing_parent_keeps_its_live_child_in_one_cell`; `a_capture_only_command_is_missing_not_finished_when_its_registration_is_unknown`; `an_incomplete_family_is_published_at_once`; `a_blocked_metadata_read_does_not_hold_the_next_read` (with 5 blocked reads, so the pool is exhausted and publication is still on time); `an_ended_invocation_leaves_the_census_after_its_cell_departs`.
   - **Not closed by this model; moved to Phase 24 now** (showrunner ruling item 5): F036's remaining publish-path work (full refresh, `Capture::take`, attribution, without a time bound); F042's full-height grant that gives rows without a painted gain (`grid.rs` ~461) and the retained packing on minimum bodies (~5554); the short tile while cells close (d80 1.87 s, t80 11.45 s); the summary cut that hides jobs when the tile is full (t40, t80); scanner CPU and a name for each scanner thread; F040's start ink held under heavy load; the shim pid shown before the cargo pid; frame cost.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — items 1, 2 and 4: the trimmed step type and the test.
- `crates/cargo-tile/src/roster.rs` — item 3 and its test.
- `crates/tui_pane/src/tiles/draw.rs` — item 6's first drawable allocation, item 7's allocation input, item 9's text alpha on the shared timing, and wherever item 5's or item 8's measured cause lands in `tui_pane` or `cargo-tile`: the repair and its test (name the file in the summary).
- `crates/tui_pane/src/tiles/constants.rs`, `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — item 9's lifecycle timing and any additive export it needs.
- `crates/cargo-tile/src/render.rs` — the F069 integration fixture (item 7), the row fade read on item 9's timing, and item 6's hidden-mid-run withdrawal test.
- `crates/cargo-tile/src/census/command_text.rs`, `crates/cargo-tile/src/constants.rs` — item 11's one option-skipping function over `CARGO_GLOBAL_VALUED_OPTIONS`, and its test.
- `crates/cargo-tile/src/cargo-capture-shim.sh`, `crates/cargo-tile/src/hook.rs` — item 12's rendered skip list and pass-through, and the rendered-shim test.
- `crates/cargo-tile/src/shim_registration/shared_capture.rs` — item 12's pass-through test.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — one Fixed line each, and one cargo-tile Fixed line for items 11 and 12.

**Workers:** 2 writers — the queue trim, admission, the shared enter and leave timing and the saturation cap all run through one grid path in `grid.rs` and `draw.rs`, with cargo-tile's roster and render callers, so they stay with one writer; items 11 and 12 touch only the census argv reader, the shim and its tests, disjoint from that path.
- `impl` — every file above except item 11 and 12's, items 1 to 10, and their tests; items 5 and 8 begin with the live diagnosis. Owns both changelogs; a `crates/cargo-tile/src/constants.rs` constant it needs goes through `impl2`, which owns that file, posted on the board.
- `impl2` — items 11 and 12: `census/command_text.rs`, `constants.rs`, `cargo-capture-shim.sh`, `hook.rs`, `shim_registration/shared_capture.rs`, and their tests; posts its changelog line on the board for `impl` to write.

**Constraints from prior phases:** Phase 36 replaces the step queue with one target layout; this phase trims the queue and does not redesign motion: the 720 ms resident step, the queue's other contents and every motion ruling in force (Phase 14: an entering or leaving piece slides; the spare-row rule binds residents and every settled frame) stay, and entering and leaving follow item 9's one timing. Phase 20's allocation solver keeps F054's protected-content and termination guarantees through item 7's cap. Phase 21: hidden groups leave at one boundary, `Roster::visible(&mut self, hidden) -> VisibleRoster` (`crates/cargo-tile/src/roster.rs` ~405), built once per frame in `render::draw` (~262); `Roster::observe` and `Roster::advance` still run on the raw roster, hidden groups included. Admission (item 6), withdrawal (item 3) and the grid's ids (`VisibleRoster::tiled_ids`) are decided on the visible view: a hidden group never enters the grid, and item 6's content row is a visible row. Once main 2a3568a4 merges in, `SemanticTheme` has a required `on_error` field: any `SemanticTheme` literal this phase writes, test fixtures included, sets it. No existing `tui_pane` public signature changes. A `tui_pane` change also runs `verify.sh test` for `cargo-handler` and `cargo-port`. No new `#[allow]` or `#[expect]`; no test takes a second.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` and `... lint cargo-tile` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; `a_command_that_ends_mid_backlog_leaves_the_grid_on_the_next_motion`, `a_new_cells_first_visible_frame_draws_a_content_row`, `a_command_hidden_mid_run_leaves_on_the_next_frame`, `an_excluded_command_after_a_valued_global_option_is_excluded`, `cargo_offline_port_passes_straight_through_the_shim`, `the_installed_shim_skips_every_valued_global_option`, the restored F069 fixture and `a_saturated_cell_keeps_no_surplus_height` red before the fix and green after; F054's two regression tests stay green; no test takes a second or more.
- Unit director's capture of a live tile with several commands ending during a backlog: each ended command's cell leaves on the next motion.
- Live shot for the checkpoint (showrunner, 2026-10-16), under a busy build load like the user's (several concurrent cargo builds, checks and tests): no cell drawn with `content rows: 0` past its close motion, and no cell whose foot counts rows draws a blank body. The design check judges it with this context.
- Change-review line (production doc's change review file): an ended command's cell leaves on the next motion, and no cell stands with a blank body.
- Change-review line: a closing cell shrinks while its text fades, with no empty-cell step.

### Phase 23 — Arguments the user chooses, and two keys that show more  · status: done

#### As-built

- `commands.default_arguments` (`DefaultArguments`, default `all-targets`, `fix`, `workspace`) is saved sorted and deduplicated. A row shows its subcommand plus each listed argument present in its argv.
- `…` marks only a cut by width or height. A hidden argument is absent and has no mark.
- cpu, mem and state columns are right-aligned by one rule (`numeric_column`).
- A wrapped command keeps a flag with its value. cargo-tile owns the break (`wrapped_command`, `command_unit_line` in render.rs), not `tui_pane`.

**Files:**
- `crates/cargo-tile/src/config.rs`, `settings.rs` — the setting and its screen entry.
- `crates/cargo-tile/src/census/command_text.rs`, `census/scan.rs` — `CommandText`, `CommandWord`, `Ancestor.command`.
- `crates/cargo-tile/src/constants.rs` — the valued-option table.
- `crates/cargo-tile/src/app.rs`, `globals.rs`, `keymap.rs`, `README.md` — the two states and keys.
- `crates/cargo-tile/src/render.rs` — `CommandView`, wrap, cut, alignment.

**Binds later work:** `COMMAND_VALUED_OPTIONS` with `ValuedOptionScope::{Global, Cargo, RustcLint}` in constants.rs is the one valued-option table; `CommandText::subcommand` reads parsed `CommandWord` values; `Ancestor.command` is a `CommandText` parsed from argv in census/scan.rs; `CommandView` in render.rs holds `ProcessHierarchy`, `ArgumentDetail`, `DefaultArguments` as independent inputs; `ARGUMENT_OMISSION_MARK` is removed; the binding ancestry test is `default_ancestry_marks_only_a_cut_name_or_walk`; keys `p` (process hierarchy) and `h` (all arguments).

**Gotchas:**
- The height cut works on parsed units (`JoinedCommandWord`, `CutCommandUnit::{Prefix, Omitted}`), never on painted characters. Whitespace boundaries are recorded before punctuation wrapping.
- A bound flag/value unit is omitted unless its cut prefix shows value text. The cut reserves the next word separator.
- nextest valued flags are not in the table. cargo as the rustc wrapper shows a bare `cargo`.

**Ruled out:** a cut rebuilt from painted text (offsets drift from argv); `SummaryDetail::Trimmed` (dead, removed).

### Phase 24 — One lifecycle for every invocation  · status: dropped (showrunner, 2026-11-06: not landed; the defects that remain live in Phases 31 and 32)

**Outcome (2026-10-25).** Phase 24 used its 3 repair rounds and one bounded step past the cap that the showrunner allowed for one regression, and its second paired live capture against main (fa651052) failed the landing rule. By the showrunner's ruling, phase 24 does not land. Its round 3 tree is on the unit branch as a plain commit, not a checkpoint, and Phases 24a and 24b start from it.
- The repair that worked: a new job waited 2.3 to 4.4 s for its first cell while rows were free (first capture, `shots41_reply.txt` CAUSES 1). The cause was the census metadata read queue: each new cargo job waited behind the reads of known lifetimes. `DetailRead` (`Queued`, `Reading`, `Ready`, `Refreshing`) now reads a new lifetime first, and a source change has its own state (`SourceChanged`), so it does not wait behind ordinary refreshes. After the repair, 64 of 64 jobs got a cell (before: 54), and the d80 first cell came in 0.43 to 2.20 s.
- Second capture, branch against main over the four pairs (`shots42_reply.txt`): rule a 2/12, rule b 1/16, rule c 0/520, rule e 0/19; never-drawn 0/0; d80 census max 0.43 s; CPU per drawn cell better than main in t40 (6.72/8.88), t80 (6.33/6.59) and d40 (10.32/11.13).
- Counts that failed: d80 CPU per drawn cell 7.57 > 6.99 (time-weighted 8.40 > 7.78); t40 rule a, rule b, F040 and F030 at 1 > 0. At t40 19.95 to 20.27 s, a cell that changed form was drawn in its old and its new form for 12 frames (the ink drop is the F040). At 23.06 to 24.10 s, a parent pid got a second cell for its job (p22k15).
- Also seen in the shots: the summary lists jobs that already have a cell (all four captures); ended cells shrink out as one-row headless cells in mid-column (d40 9.00); in t80 one pid row is painted 3 to 4 times in one cell; two header forms (`pid parent command` and `pid command`) show at once.
- The showrunner's reading: the t40 breaks, the summary double, the repeated pid rows, the mixed cell forms and the first-cell wait have one cause. One invocation can be in two places at once (the summary and a cell, or two cells for two of its pids). That is this phase's goal, and the types do not hold it yet. Phase 24a takes it; Phase 24b takes the ended cell that leaves as a headless row.
- Evidence: `shots41_reply.txt` (first capture) and `shots42_reply.txt` (second capture) in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad`, with the shots in its `ph24` and `ph24b` folders.

**Depends on:** Phase 21, Phase 22 (its Work Order and item 14's state model; its code did not land), Phase 23

#### Work Order

**Goal:** One command invocation has one owner record and one state, from discovery to the end of its cell, and the paired live capture against main is equal or better on every count. Phase 22 did not land (its Outcome), so this phase carries Phase 22's Spec items 1 to 13, their tests and their gate lines, and builds them on Phase 22 item 14's state model. Before any code, the model is extended to the failures that the round 16 and round 18 captures showed and to the closure 18 findings. The round 16, 17 and 18 trees (`/tmp/claude/delegate/03f9b80e-dbe5-47c6-b29e-a7e6587ab721/phases/22/trees`) are patches to start from, not landed code.

**Spec:**

1. **Extend the state model before any code.** Start from Phase 22 item 14 (refer to it; do not copy it), and add only these parts:
   - **The summary cut.** Under foreign load (a foreign session fills the tile with cargo-example cells), each live command that has no cell is listed in the summary before the commands that have a cell, so no live command is drawn nowhere. Evidence: round 18 d80 census up to 6.01 s with 10 jobs over 1.0 s, and 17 jobs never drawn (d80 1, d40 16); round 16 tree d80, jobs p22k09 to k13 waited 1.78 to 2.25 s while the summary was cut to one row of foreign commands (`shots39_reply.txt` CAUSES 1; `shots40_reply.txt` CAUSES 3 and 7). The model names the type that makes "live, no cell, and no summary row" impossible.
   - **The start ink of a new cell (F040).** The fade-in starts on the frame of the cell's first paint, so a new cell never paints frames at its start ink before its fade-in. Evidence: F040 7 in round 18 and 13 in the round 16 tree, all at frame times of 20 to 50 ms under load (`shots39_reply.txt` CAUSES 2; `shots40_reply.txt` CAUSES 6). The same rule covers the ended cell: its ended fade starts on the frame its end is seen, and the fade-in stops there (`shots39_reply.txt` CAUSES 3).
   - **The relayout eviction (F045).** A growing cell or a summary flip never pushes a live cell out of the tile. A frame's layout keeps each drawn live cell, or moves it to the summary only by Phase 22 item 13's capacity rule. The summary is measured after its cut, never with its full row set. Evidence: round 18 t80, p22k07 gone for 5 frames; round 16 tree d80, the summary took the full column for one frame twice (7 live cells gone each time) and p22k09 was evicted for 0.87 s; t40, a foreign cell grew with blank rows and pushed p22k02 out (`shots39_reply.txt` CAUSES 4; `shots40_reply.txt` CAUSES 1, 2 and 4).
   - **Each closure 18 P1 maps to the type rule that makes it impossible** (`/tmp/claude/delegate/03f9b80e-dbe5-47c6-b29e-a7e6587ab721/phases/22/review_findings_18.txt`):
     - F030: roster groups are a projection of `Invocations` only; a forwarding join is a published transition, never an omission.
     - F045: presentation belongs to each painted row span, not to the lead, so an ended child drawn in its lead's cell keeps its full fade.
     - F049: presentation is an enum (`NotPresented`, `Presented`, `Withdrawn`, `Departed`); hiding is a withdrawal that completes the receipt.
     - F050: the join takes a lifetime match proof; a terminal identity stays terminal after its record is removed.
     - F051: painted row spans carry their `InvocationId`; no ownership is parsed from painted text.
     - F052 (minor): admission keeps its `AdmissibleInvocation`, and `DrawableCell::prepare` takes it.

   **Done before dispatch:** the extended model is written and the showrunner approved it (2026-10-21): `/tmp/claude/delegate/03f9b80e-dbe5-47c6-b29e-a7e6587ab721/phases/24/state_model_extension.md`. Build to it; its rulings replace its open questions:
   - Ruling 1 (A): the summary holds a row for each `Listed` command; the newest cells move to the summary to make room.
   - Ruling 2 (B): `TileDemands.summary` changes to `SummaryDemand`, so a caller that passes the full row set does not compile. This phase updates every `tui_pane` caller (cargo-handler and cargo-tile). New `TileCells` methods may be additive; `admission` has no default implementation, because a default would give an `AdmissibleCell` without its proof. A default is allowed only where it cannot make a proof.
   - Ruling 3 (A): entry uses ease-out for text and geometry together; leaving keeps ease-in.
   - Ruling 4 (A): `CARGO_TILE_FRAME_LOG` records the publication time of each end, and the ended ink rise counts from that time.
   - Ruling 5 (A): the capture column F045 becomes "lost live cell"; F045 stays the closure finding.
   - Addition: a test proves the claim in `RequiredMinima::check` that greedy column-major fill is exact for ordered packing: for all small cases (up to 6 cells, 1 to 3 columns, small heights), `check` agrees with an exhaustive search over the ordered packings. It takes under 0.1 s.
2. **A foreign-load fixture that runs in under one second.** The fixture reproduces the foreign load: a foreign session fills the tile with cargo-example cells while short jobs start and end. It checks the d40 and d80 failures (never-drawn jobs, census time, F040 and F045) through the census, the roster and rendered buffers. Each repair round runs it before a live capture. It fails on the round 18 tree's patch and passes on the candidate.
3. **The items that Phase 22 item 14 moved to later:**
   - F036's remaining publish-path work: the full process refresh, `Capture::take` and attribution get a time bound, each with its measured cost.
   - F042: the full-height grant that gives rows without a painted gain (`grid.rs` ~461), and the retained packing on minimum bodies (~5554).
   - The short tile while cells close (d80 1.87 s, t80 11.45 s).
   - Scanner CPU, and a name for each scanner thread.
   - F040 under heavy load: seven new cells held their start ink 0.13 to 0.17 s before the fade-in (round 16 capture, t80 at load 71; main md80: one). It closes with item 1's start-ink rule.
   - The shim pid shown before the cargo pid: two cells showed the shim pid for about 0.3 s, then the cargo pid (round 15 capture, t80). A cell shows the cargo pid from its first paint.
   - Frame cost (showrunner ruling 2026-10-18, round 8 release capture: slow frames took 24 to 26 ms to draw in busy captures, main 0.2 ms). First put the draw time of each frame in `CARGO_TILE_FRAME_LOG` (today it gives one summary per 120 frames and the frames over 33 ms). Then remove the repeated per-frame collections, identity searches and lookups (closure 8 item 6). Exit: in busy captures, no frame draws for more than 4 ms in a release build. CPU per drawn cell is no worse than main (round 16 tree: d80 19.65 against 16.16, d40 21.46 against 17.54, from a fixed cost that does not change with the cell count; `shots40_reply.txt` CAUSES 8).

**Landing rule:** In the paired live capture, the branch is equal to or better than main on every count of the BRANCH AGAINST MAIN table in `shots39_reply.txt` (rule a, rule b, rule c, rule e, content rows 0, blank body, F041, F044, F040, F045, F030, F035, F038, admission, longest free wait, never-drawn, census max), and on CPU per drawn cell. Also: in d80 each job is drawn within 1 s (census), and never-drawn is 0. At most 3 repair rounds. The state model is reviewed before code.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — Phase 22 items 1, 2 and 4, and item 3's F042 grant and packing.
- `crates/cargo-tile/src/roster.rs` — Phase 22 item 3, and item 1's roster projection (F030).
- `crates/tui_pane/src/tiles/draw.rs` — Phase 22 items 5 to 9, and item 1's start ink and eviction rules.
- `crates/tui_pane/src/tiles/constants.rs`, `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — Phase 22 item 9's lifecycle timing and any additive export.
- `crates/cargo-tile/src/render.rs` — Phase 22 items 6, 7 and 9, item 1's summary cut, and item 3's per-frame draw cost.
- `crates/cargo-tile/src/census/command_text.rs`, `crates/cargo-tile/src/constants.rs` — Phase 22 item 11 over the `Global` scope of `COMMAND_VALUED_OPTIONS`, and its test.
- `crates/cargo-tile/src/cargo-capture-shim.sh`, `crates/cargo-tile/src/hook.rs` — Phase 22 item 12's rendered skip list and pass-through.
- `crates/cargo-tile/src/census/process_identity.rs` — F050's lifetime match proof.
- `crates/cargo-tile/src/census/mod.rs`, `crates/cargo-tile/src/census/scan.rs` — item 3's scanner threads, their names and the publish-path time bounds.
- `crates/cargo-tile/src/probe.rs` — item 3's per-frame draw time in `CARGO_TILE_FRAME_LOG`.
- `crates/cargo-tile/src/shim_registration/shared_capture.rs`, `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — Phase 22 item 12's pass-through test and the item 2 foreign-load fixture.
- `crates/cargo-tile/src/tiles.rs`, `crates/cargo-handler/src/tiles.rs`, `crates/cargo-handler/src/render.rs` — ruling 2's `SummaryDemand` callers.
- `crates/cargo-handler/CHANGELOG.md` — one Changed line for the summary demand.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — one Fixed line each, and one cargo-tile Fixed line for Phase 22 items 11 and 12.
- Added during the phase: `.config/nextest.toml`; `crates/cargo-tile/Cargo.toml` and `crates/tui_pane/Cargo.toml` (the `test-clock` feature); `crates/cargo-tile/tests/support/foreign_load.rs`, `crates/cargo-tile/tests/cli_lifecycle.rs`, `crates/cargo-tile/tests/shim_modes.rs`; `crates/cargo-tile/src/shim_registration/wire.rs`, `crates/cargo-tile/src/shim_registration/reader_scenario.py`; `crates/cargo-tile/src/terminal.rs`, `crates/cargo-tile/src/birth_stamp/process_lifetime.rs`, `crates/cargo-tile/src/census/command_text.rs`, `crates/cargo-tile/src/census/invocation_cpu_accounting.rs`; `crates/cargo-handler/src/render/role_title_tests.rs`, `crates/cargo-handler/src/render/accounts_grid_tests.rs`.

**Workers:** as Phase 22 (`impl` for the lifecycle path, `impl2` for Phase 22 items 11 and 12), plus `test`, which owns the item 2 fixture and the paired live captures and writes no product file.

**Constraints from prior phases:** Phase 22's Constraints hold, with one approved exception: ruling 2 changes `TileDemands.summary` to `SummaryDemand`, a `tui_pane` public signature change, and this phase updates its callers. Phase 23's argument setting, `…` rule and keys are on the branch; this phase keeps them and their tests. Phase 23 replaced the table Phase 22 items 11 and 12 name: `CARGO_GLOBAL_VALUED_OPTIONS` is now `COMMAND_VALUED_OPTIONS` (`constants.rs` ~509), each entry tagged with a `ValuedOptionScope` (`Global`, `Cargo`, `RustcLint`); retained subcommands come from parsed `CommandWord` values, and `Ancestor.command` is a `CommandText` parsed from argv (`census/scan.rs`). Read items 11 and 12 with that table: the free argv `subcommand` (`command_text.rs` ~624) and the shim's rendered skip list reuse its `Global` scope, with no second table. Phase 36 replaces the step queue later and keeps this phase's tests as binding regressions. A `tui_pane` change also runs `verify.sh test` for `cargo-handler` and `cargo-port`. No new `#[allow]` or `#[expect]`; no test takes a second.

**Acceptance gate:**
- The ordered-packing test agrees with the exhaustive search and runs under 0.1 s.
- `verify.sh test` for `tui_pane`, `cargo-tile`, `cargo-handler` and `cargo-port`, green.
- Phase 22's gate lines, its two change-review lines included; Phase 22 item 14's tests red before the fix and green after; the item 2 fixture under 1 s, red on the round 18 tree and green on the candidate.
- The paired live capture against main meets the landing rule.
- Change-review line: under a full tile, a live command with no cell is in the summary.
- Change-review line: a new cell fades in from its first frame, and no live cell leaves the tile when another cell grows.

### Phase 25 — One invocation, one place  · status: dropped (showrunner, 2026-11-06: not landed; Phase 28, its rebuild, was dropped because the user did not ask for it)

**Depends on:** Phase 24 (its round 3 tree, its Outcome and its state model)

#### Work Order

**Goal:** One invocation is drawn in one place at a time: in the summary or in one cell, never in both, never in two cells, and each of its pid rows is painted once. Phase 24's goal holds; this phase makes the types hold it. The showrunner's split ruling (2026-10-25) names one cause for the Phase 24 t40 breaks, the summary double, the repeated pid rows, the mixed cell forms and the first-cell wait: one invocation can be in two places at once.

**Spec:**

1. **Extend the state model before any code, and send it to the showrunner for review.** Extend `/tmp/claude/delegate/03f9b80e-dbe5-47c6-b29e-a7e6587ab721/phases/24/state_model_extension.md`. It names the type that makes each of these impossible:
   - **An invocation in the summary and in a cell at the same time.** Placement is one enum for each `InvocationId` (for example `Listed`, `Celled`, `Departing`), and the summary and the tile both project from it.
   - **Two cells for one invocation.** The parent pid, the shim pid and the cargo child pid resolve to one `InvocationId` before admission. A pid change (shim to cargo, alias to child) updates the record and never opens a cell.
   - **One pid row painted more than once in a cell.** Row spans key on (`InvocationId`, pid).
   - **Two header forms at once** (`pid parent command` and `pid command`). The header form comes from one value for each frame layout, so a form change replaces the cells and never adds a second set.

   **Done before dispatch:** the extended model is written as section (e) of the state model file, and the showrunner approved it with its 7 recommended answers (2026-10-25). Build to it, with the showrunner's three notes:
   - Item 5: with exclusive placement, the time a job stays `Listed` while rows are free (0.1 to 1.8 s in the Phase 24 capture; p22k00: shim pid 1617011 listed, cell at 4.36 s with cargo pid 1617240) becomes the visible wait. The model states its cause; the expected cause is the time until the refresh finds the cargo child pid. The capture reports the `Listed` to `Celled` time with free rows for each job. It is not a landing count. If it is over 0.5 s and its cause is the refresh cadence, Phase 31 takes a targeted read of the registered pid's children when a new registration comes in.
   - Item 6: before the profile, diff the refresh path against main. If the branch makes a new `System` and does a full refresh on each pass (`census/scan.rs` ~862, ~1327) and main does not, that is the first suspect for the CPU per drawn cell difference. Then profile by function.
   - Item 4: one header form for each frame, chosen from the narrowest cell, is correct (one style on the screen). The summary keeps its own form.
2. **A cell opens at admission with no CPU sample.** CPU is an enum (`AwaitingFirstSample`, `Sampled`), and admission never waits for the first sample.
3. **Tests.** One fixture under 1 s for each Phase 24 t40 case: a form change while a job ends (t40 19.95 to 20.27 s), and a parent pid that gets a second cell (t40 23.06 to 24.10 s). Each fails on the Phase 24 round 3 tree and passes after.

**Landing rule:** Phase 24's landing rule, unchanged, CPU per drawn cell included (Phase 24 d80: 7.57 against main 6.99). If the duplicate cells are not that cost, the phase names the cost. The state model is reviewed before code. At most 3 repair rounds.

**Files:** Phase 24's Files, the paths added during Phase 24 included.

**Workers:** as Phase 24: `impl` for the lifecycle path, `impl2` for the census, and `test` for the fixtures; a capture helper takes the paired live captures.

**Constraints from prior phases:** Phase 24's Constraints hold. Its round 3 repairs and their tests stay, as do Phase 24's `DetailRead` read order and its tests. No new `#[allow]` or `#[expect]`; no test takes a second.

**Acceptance gate:**
- The extended model is written and the showrunner reviewed it before code.
- The two t40 fixtures fail before the fix and pass after; each takes under 1 s.
- `verify.sh test` for `tui_pane`, `cargo-tile`, `cargo-handler` and `cargo-port`, green.
- The paired live capture against main meets the landing rule.
- Change-review line: no invocation is in the summary and in a cell in the same frame, and no invocation has two cells.

#### Outcome

Not landed (showrunner ruling 2026-10-26, after 3 repair rounds). The round 3 tree is commit `d22c672b` and is the base of the next lifecycle phase.
- Round 1 capture (shots43, against main `c9142b19`): every job got a cell (64 of 64 against 25 of 64), the d80 census max was 0.50 s and no job showed in the summary and a cell at once. The two-pass frame draw made the layout flip every frame (d80, 181 frames), the tile ran past the screen, ended jobs showed in two cells, and the alias parent kept its own cell.
- Round 3 capture (shots44, against main `595145da`): those faults are gone, d80 and d40 have 0 on every rule count, the d80 census max is 0.39 s, and never-drawn is 0 (main 4). Still worse than main: the rows-cut mark with free rows (0.4-1.4 s per pair), a closing ended cell puts its job back in the summary for 1 frame and the cells below move (16 times), t80 admission 7 against 10 (an uncut 30-row foreign cell takes a column), CPU per drawn cell (t80 28.80 against 15.68, d40 18.95 against 15.69), and frame draw 2-4x slower in t40 and t80.

### Phase 26 — Reader and named slow tests hold under load  · status: done

#### As-built

- The `reader_scenarios` tests and the named `cargo-tile` and `tui_pane` slow tests pass under full-suite load. The work is on main, not on the Phase 25 tree, because that tree did not land.
- On macOS, `reader_script_result` (`reader_scenarios.rs`) runs a staged copy of the reader executable from the fixture temp directory. Foundation reads the executable directory when it first reads the system appearance; the staged copy keeps that read away from the build artifacts and removes the reader startup cost.
- `reader_scenario.py` waits for the table headers and for the native pid of each command. `cpu_observation_is_complete` requires the sustained samples to span one reporting window. The CPU charge check is main's check with a floor of 1%.
- CPU smoothing uses the time between samples. `CpuSmoothingSample { AwaitingFirstSample, Observed(Instant) }` in `InvocationCpuAccounting` holds the time of the previous sample, apart from `CpuPublication`; `CensusCadence::smoothing_alpha(elapsed)` computes the weight, and the first sample uses the poll interval. A late scan keeps the configured smoothing window (`a_late_cpu_scan_keeps_the_configured_smoothing_window`).
- The `tui_pane` motion and draw tests sample the frames at each boundary of a geometry change, not every frame, and read buffer rows as slices. The 13 named tests take 0.02-0.34 s alone. The product resolve code is the same as main: `measure_moving_height_rows` and `select_moving_heights` each call `drawing_at_with_height_snap`.
- The panic report test writes a known frame and does not resolve a native backtrace; `append_panic_report` takes any `Display` backtrace.

**Files:**
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — `CpuSmoothingSample` and the smoothing weight from elapsed time.
- `crates/cargo-tile/src/census/scan.rs` — the late-scan smoothing test.
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — the staged reader copy on macOS.
- `crates/cargo-tile/src/shim_registration/reader_scenario.py` — the header and pid waits, the sample-span completion check, and the CPU charge check with a 1% floor.
- `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs` — boundary-sampled motion and draw tests; `drawing_from_destination` and `distinct_motion_steps_for_test` serve the test sampling.
- `crates/tui_pane/src/runner/lifecycle.rs` — `append_panic_report` with a `Display` backtrace.

**Binds later work:** In 2 of 5 full Mac runs, the reader charged about 3x the CPU of the native counters; the cause is open and belongs to "The CPU a row shows is the CPU its processes used", which starts from the strict native-counter check saved as `phases/26/strict_cpu_check.patch` in the session. "Phase 25's one-place work returns on main" rebuilds from `d22c672b` and `phases/26/on_24a_round3.patch` and keeps the reader safeguards above. The `tui_pane` sampling helpers and their assertions stay.

**Gotchas:**
- Under load, frames that drain together can end a CPU wait with a sample span of a few ms. Completion needs a minimum sample span, not a sample count.
- A CPU charge floor of 10% failed on a correct product at load 33.
- A test time taken alone under load is an upper bound on its quiet time.
- A tree reset to main also resets the plan doc to main's version.

**Ruled out:**
- The strict native-counter CPU comparison in the reader check, until its cause is measured — it failed 2 of 5 full Mac runs.
- Grid destination reuse in `tui_pane` — no test shows it equal to a fresh resolve after the row measurements.
- A longer time limit for a slow test.

### Phase 27 — The CPU a row shows is the CPU its processes used  · status: done

#### As-built

- The CPU value of the product is correct, and `census/` accounting behavior is the same as before. The earlier strict check failed because it compared a held, smoothed table reading with a later native rate, and it used the time when `reader_scenario.py` drained a frame, not the time when the reader published it.
- The strict check in `reader_scenario.py` calculates the expected value from the native counters with the rules of the product. `NativeCpuHistory` and `NativeCpuOracle` apply the retention of each platform, the elapsed-weighted smoothing, the first sample taken whole and the hold until the report deadline, on integer-nanosecond scan and publication times. `UniqueProcessContributions` refuses a process charged two times. `FixtureCpuOwnership` checks the exact charged pid set against an independent `ps` parent snapshot. `stopped_compiler_cpu` stops the compiler with SIGSTOP and compares its counter with an independent kernel read: `/proc/PID/stat` with `SC_CLK_TCK` on Linux, `proc_pid_rusage` `RUSAGE_INFO_V0` with `mach_timebase_info` on macOS. Named Python types hold the oracle states: `UnavailableCpuReading`, `CpuPercent`, `AwaitingCpuBaseline`, `CpuCounterBaseline`, `CpuScanClock`, `CpuPublicationClock`, `ObservedCpuTime`, `UnstampedCpuFrame`, `CpuFrameStamp`.
- A test-only recorder, `census/cpu_observation.rs` (`#[cfg(test)]`), gives the oracle the scan ids and the frame publication times. `CpuScanPublication::{Unobserved, Observed(u64)}` moves along this chain: `cpu_observation::work` (in the attribution loop of `census/scan.rs`, before accounting uses the counters) → `InvocationCpuAccounting::observe_cpu` → `InvocationMeasurements::cpu_scan_publication` → `Scan::cpu_scan_publication` → `cpu_observation::accept` in `drain_scans` → `cpu_observation::publish_frame()` in `App::draw`. `publish_frame` writes OSC 777 `cargo-tile-cpu;<scan>;<ns>` to stdout before the backend writes the frame. The recorder writes `READER_CPU_OBSERVATION_FILE` (`cpu-observation.tsv`); the Python file names the same string. Only the cpu-cache-server reader child calls `start()`, and the normal binary has no recorder.
- Five tests on supplied counters and times in `reader_scenarios.rs`, each under 1 s on the Mac: `reader_cpu_oracle_applies_smoothing_and_publication_to_changing_load`, `reader_cpu_oracle_keeps_publication_times_when_frames_are_drained_late`, `reader_cpu_oracle_rejects_a_compiler_counter_charged_twice`, `reader_cpu_oracle_checks_the_compiler_counter_against_the_kernel`, `reader_cpu_oracle_publishes_at_each_exact_report_deadline`.
- Reader cleanup sends SIGKILL and closes the PTY before `waitpid`, so it does not wait until the 20 s scenario limit. The CPU fixture waits for the pid files of both clients before it releases the reader.

**Files:**
- `crates/cargo-tile/src/census/cpu_observation.rs` — the test-only recorder and `CpuScanPublication`.
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — `observe_cpu` and the `cpu_scan_publication` field (`cfg(test)`).
- `crates/cargo-tile/src/census/scan.rs` — the `work` and `observe_cpu` hooks.
- `crates/cargo-tile/src/census/mod.rs` — the `cpu_observation` module (`cfg(test)`).
- `crates/cargo-tile/src/terminal.rs` — `Scan::cpu_scan_publication` and the `accept` call in `drain_scans`.
- `crates/cargo-tile/src/app.rs` — the `publish_frame` call in `App::draw`.
- `crates/cargo-tile/src/constants.rs` — `READER_CPU_OBSERVATION_FILE` (`cfg(test)`).
- `crates/cargo-tile/src/shim_registration/reader_scenario.py`, `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — the oracle, its tests and the cleanup and fixture changes.

**Binds later work:**
- The recorder chain and all its hooks stay through the rebuild of `d22c672b` in "Phase 25's one-place work returns on main"; the measurement stamp goes through the `AttributedInvocations::for_census` reuse.
- The oracle retention branch (`NativeCpuHistory.sample`, `CounterRetention.REAPED_CHILDREN`) is a copy of `RetainedSubtreeCpuTime::update` (`self.total.max(current.values().sum())`) and asserts the `accumulated` value of the product within 1e-7. A change to the product rule in "A nextest run shows its tests, and CPU counts only what each process adds" changes the oracle copy in the same change.
- The OSC 777 frame stamp stays tied to a drawn frame. An idle tile that draws no frame writes no stamp ("Every cell moves toward one target layout").
- The live cache-server reader test takes 7.5 s focused and 15.439 s in the full Mac suite at load 27-34. The SIGSTOP kernel check waits for two later scan records. "A nextest run shows its tests, and CPU counts only what each process adds" owns this run time.

**Gotchas:**
- Linux `ps` drops fractional seconds, and Mac `ps` rounds to 0.01 s. `proc_pid_rusage` gives Mach ticks (timebase 125/3 on the Mac), not ns. Read the kernel directly.
- Float subtraction misses exact report deadlines (0.15 - 0.1 < 0.05). Compare integer nanoseconds.
- Reader scenarios run the reader child of the test binary (`reader_child` in `reader_scenarios.rs`), so `cfg(test)` hooks reach them.

**Ruled out:**
- A product accounting repair — the measurement showed no product defect.
- An OSC record in normal product output — the recorder is test-only by construction.
- `ps` as the independent CPU source — its rounding gave false failures.

### Phase 28 — Phase 25's one-place work returns on main  · status: dropped (showrunner, 2026-10-29: the user did not ask for it; four tries broke live fill)

> **2026-10-29 ruling.** Phase 28 is past its 3 repair rounds and is worse than main on live fill (live gate at 393 x 89 on both machines: columns end above the foot in 73-88% of frames, main about 1%). It does not land. This branch is set back to main 655c7948 by a revert commit; phases 30, 32, 35 and 36 are ported onto main and checkpointed one by one. The phase 28 tree with phases 29-36 is kept at `refs/tile-unit/p28-snapshot-1322`. The redo works in a scratch tree made from that snapshot: live bisect, a red test from the worst live frame, then a fix of the cause; no install until the live gate matches main.
>
> **Landing rule for the redo.** The revert enters main's history at this unit's first checkpoint after the ruling. So the redo must land as NEW commits ported on top of main. Never merge `refs/tile-unit/p28-snapshot-1322`, or any branch that holds the old phase 28 commits (e21d5b93 and the merges after it): git keeps the revert and silently drops phase 28's changes.

**Outcome (2026-10-28).** Phase 28 was rebuilt on main and used its 3 repair rounds. The closure reviews accepted all 130 review findings. Main 3f00dce9 (handler phase 27) is merged into the tree. The tree passes the four full Mac suites (`tui_pane` 1208, `cargo-tile` 1284, `cargo-handler` 719, `cargo-port` 1082) and the lint pass on the whole tree. The paired live capture against main 3f00dce9 (shots45, four pairs; 1-minute load up to 524) gives LAND no by the landing rule. The three-round rule is the user's, thus the showrunner takes the landing exception to the user. Phase 28 does no new repair work.
- Better than main: rule a/b/c 1/1/0 against 37/42/802; content-rows-0 and blank body 0 against 774/802; F040 0 against 34, F044 0 against 18, F045 0 against 21. One place: no frame shows a job in the summary and in a cell (main: 22 to 29 stretches in each capture). Admission 64/64 against 56/64; never-drawn 0 against 6; d80 census max 0.37 s against 2.07 s. All the Phase 25 faults are gone.
- Worse than main: rows cut while free rows exist, 0.69 to 1.36 s in each pair (main 0); CPU per drawn cell 1.2 to 3.0 times main (the branch draws 99 to 122 fps against 46 to 111); t40 longest free wait 1.60 s against 0.09 s; in t80, one job in two cells for 2.86 s (F030 1); frames over 4 ms in all four captures.
- Main 3f00dce9 panicked in t80 in 3 of 3 runs at `tui_pane` `grid.rs:5536` (a clamp with min > max). The branch did not panic in 13 runs.
- Handler test `narrow_growth_reports_fixture_counts`: its 20-group row at width 34 now expects 2 painted groups, because under phase 28 a placement is a painted cell. The showrunner accepted this change.
- `tui_pane` public items that this phase adds (`cargo-tile` or `cargo-handler` production code uses each one): `SummaryDemand`, `AdmissibleCell`, `CellDeparture`, `CellDepartureReceipt`, `CellLifecycleTiming`, `EndedCellDeparture`, `FrameLayoutPreparation`, `FramePlacement`, `NotAdmissible`, `PaintedRowSpan`, `PresentedRowSpan`.
- `.config/nextest.toml` now sets a default `slow-timeout` of 15 s with `terminate-after` 2: nextest stops a test after 30 s.

**Depends on:** Phase 27.

#### Work Order

**Goal:** Phase 25's work (one invocation in one place, commit `d22c672b`) is rebuilt on current main and lands by the Phase 24 landing rule. Phase 26 moved off that tree (showrunner, 2026-10-27) because it did not land.

**Spec:**
1. **State model first.** Phase 24's state model with its section (e) is written again for the current main tree in the plan before code, and the showrunner reviews it.
2. **Rebuild** from `d22c672b`'s changes and the saved seat work `phases/26/on_24a_round3.patch` in the session (render capacity cache and borrows, grid and draw edits, `foreign_load` caps, `one_place`, reader script), less the two parts that Constraints exclude: the grid destination reuse and the strict CPU comparison.
3. **The slow tests that exist only on that tree** take under 1 s alone and pass under full-suite load: `foreign_load::a_full_tile_under_foreign_load_draws_every_job_at_40_columns` and `_at_80_columns`, `render::tests::scanned_commands_fill_free_rows_beside_a_tall_summary`, `render::tests::rule_a_ended_cell_is_offered_more_rows`, `tiles::grid::tests::a_removal_whose_new_widths_change_demands_still_moves_once`, `a_header_form_change_at_an_end_keeps_one_cell_per_invocation`.

**State model (Spec item 1; approved by the showrunner 2026-10-27):**

##### Phase 28: state model for the rebuild on main

Baseline: `remove-running-section-tile`, HEAD `64c28f33`, main `e9375e42` plus Phase 27. This document covers Spec item 1 only. It does not authorize code changes. The order of design choices is Simple, Fast, Beautiful, with correctness required in each choice.

##### Sources and reference rules

The sources are the full Phase 28 Work Order and the Phase 24, 25, 26 and 27 records in `docs/cargo-port/remove-running-section-tile-fixes.md`; Phase 24's `state_model_extension.md`, including section (e) and its approved answers; the full tree of `d22c672b`; `phases/26/on_24a_round3.patch`; and the current tree.

In this document, `CT` means `crates/cargo-tile/src`, and `TP` means `crates/tui_pane/src/tiles`. A reference without a commit names the current tree. `OLD path:line` names a type in the full `d22c672b` tree. `ADD` names a type or contract to add in the rebuild; an OLD reference is its source, not a claim that it exists on main. Line numbers are for these two fixed trees. The Impossible columns are contracts for private constructors and transitions. Types cannot establish external kernel facts by themselves; the observation adapters must supply those facts, and behavior tests must check their use.

Phases 24 and 25 did not land. Their types are rebuild sources. They are not current-main guarantees. The Phase 25 outcome also shows faults that the rebuild must not retain: a completed cell departure returned to the summary for one frame; optional tall bodies reduced admission; some rows remained cut with free rows; draw cost increased. The saved patch supplies measured cost changes. It does not prove the landing rule.

The owners are separate:

1. Census owns identity and invocation lifetime.
2. A publication carries those decisions to the roster.
3. The roster projects rows; it does not decide that an invocation ended.
4. One committed frame owns paint location and row assignment.
5. Presentation holds paint receipts. It does not decide identity or location.
6. The grid owns geometry and motion clocks. CPU accounting owns measurement clocks.

##### What main holds today

| State or value | Current Rust owner | Trigger and transition | What the current type excludes, and what it does not exclude |
|---|---|---|---|
| Captured invocation; process invocation | `InvocationId::{Captured, Process}`, `CT/census/process_identity.rs:77`; `RunId`, line 86 | Verified capture metadata selects the captured id. A native observation supplies a process id. | The two id forms are distinct. There is no resolved execution owner that proves they cannot represent the same execution. |
| Known native lifetime; unread native lifetime | `ProcessIdentity::{Known, Unavailable}`, `CT/census/process_identity.rs:101`; `LifetimeEvidence::{Available, Unavailable}`, `CT/birth_stamp/process_lifetime.rs:43` | `ProcessIdentities::observe`, `CT/census/process_identity.rs:126`, replaces the present map on each scan. Continuous unread observations retain an unread token. A readable birth supplies a Known id. | Unavailable is not a measured birth. Main does not retain missing-process identity or terminal bindings as the old tree did. |
| Read process field; unavailable field | `ProcessField::{Observed, Unavailable}`, `CT/census/scan.rs:726`; `DetailPresence`, line 737; `ProcessObservation`, line 744 | Discovery is followed by synchronous detail reads and an overlay. | A field read failure differs from a value. There is no queued/reading/refreshing worker state on main. |
| Scan-local invocation row and family | `CargoProcess`, `CT/census/scan.rs:340`; `CargoGroup`, line 431; `Census`, line 929 | `scan`, line 610, refreshes the retained `System`, reads details, prepares identity, attributes CPU and publishes groups in that call. | These structs carry row values. They do not carry a persistent `Live/Missing/Ended` record or an admission proof. |
| Live row; finished row | `RowLifetime::{Live, Finished(Instant)}`, `CT/roster.rs:33`, inside `TrackedRow`, line 61 | `refresh`, line 111, sets Live and clears fade. `finish`, line 119, records the first omission time. `Roster::observe`, line 429, finishes absent groups; `TrackedGroup::refresh`, line 213, finishes absent children. | The enum prevents two lifetime variants in one row. It does not prevent a finished row being refreshed Live, nor does omission prove an execution end. |
| Family rows | `TrackedGroup`, `CT/roster.rs:169`; `Roster`, line 346; `VisibleRoster`, line 556 | New groups append; matching groups refresh; fade advancement removes finished rows. Hidden commands are removed from the visible view. | The Vec-based projection has no exclusive process-row owner across live and fading groups. Hiding has no census departure receipt. |
| Present, arriving, departing geometry | `CellTransition`, `TP/grid.rs:989`; `GridMotion::{Settled, Moving}`, line 1085; `Step`, line 1180; `TileGrid`, line 1198 | A demand or viewport change creates a geometric transition; motion advances to its target. | These are geometry states. They are not `Waiting/Entering/Leaving` invocation lifetime states and do not require a census end proof. |
| Summary and command demand | `TileDemands`, `TP/grid.rs:110`, with `summary: usize`; `TileDrawing`, line 214 | `Cells::prepare_frame`, `CT/render.rs:463`, selects table policy; demands and row probes drive layout. `summary_rows`, line 1584, reads the visible roster. | Main separates summary and command table grids, but has no committed exclusive placement map or summary minimum for the Listed complement. |
| Prepared rows and column layouts | `Cells`, `CT/render.rs:384`; `FrameColumnGrids`, line 2890; `FrameTableColumnGrid`, line 3202; `CachedTableLayout`, line 3814 | A frame builds intrinsic columns. Row caches and per-table layouts answer demand, probe and paint calls. | Cache storage is not proof of exclusive placement, a common final command form, or a complete cache key. |

The rebuild replaces the omission-based lifetime path. It does not add a second lifetime map beside it. Current `RowProvenance` (`CT/census/scan.rs:269`), `CaptureMembership` (`process_identity.rs:164`), `VisibleParent` (line 175), and `AncestryWalk` (`scan.rs:446`) remain distinct facts about source, parent and ancestry. None grants execution identity by itself.

##### Base model: identity, publication and lifetime

##### Process identity and execution binding

ADD the following private census types in `CT/census/scan.rs`. Their sources are OLD `ResolvedInvocationId:1643`, `InvocationProcessBindings:2010`, `InvocationHandover:3008`, and `LiveCaptureBinding:2937`. Main has only the raw ids above.

| Binding state | Holder to add | Transition and trigger | Impossible state |
|---|---|---|---|
| Executor | `InvocationProcessBinding::Executor` | A process is proved to execute one invocation. A verified representative change updates this binding and the row values. | One process subject cannot have two active canonical owners in the private binding map. |
| Forwarder | `InvocationProcessBinding::Forwarder` | A verified parent/child forwarding edge binds the subject to the executor's owner. A later source change retains that relation. | An unread parent or a capture-only interval cannot by itself split the execution into two cell requests. |
| Independent execution | `InvocationProcessBinding::IndependentExecution` | Positive evidence proves a real independent cargo execution or branch. | Common argv, directory or pid number cannot prove a join. Genuine independent children retain distinct ids. |
| Active binding; ended binding | `ProcessBindingState::{Active, Ended}` (OLD line 2003) | A resolved end transfers the subject to Ended. A different proved lifetime can create a new active binding. | A terminal subject cannot return as a new owner after its old record is removed. |
| Resolved execution id | `ResolvedInvocationId`, constructed only by the bindings | Resolution succeeds after the applicable binding proof. Publication, row keys and admission use this result. | A raw pid-derived id cannot bypass the resolver and request another cell. |

Keep native identity separate from invocation identity. ADD `LifetimeMatch` and `LifetimeContradiction` from OLD `CT/census/process_identity.rs:103,108`. A native/capture lifetime adoption consumes a match for the same subject. A failed lifetime read proves neither a match nor a replacement.

There is one additional valid proof. In one scan, a verified shim parent link can bind a captured invocation to a cargo pid that is also live in a process-resolved record. Both live records with that pid name the same process. ADD `LiveCaptureBinding` to carry the current registration, matching shim birth, current forwarding edge and live co-presence. The captured invocation owns the process from that scan onward, including when the cargo birth is Unavailable. Publish the old process record as Joined, not as its own terminal end. An old terminal record with no live overlap is not eligible. This preserves native reuse; it is not a pid-only join.

ADD `TerminalIdentities` (OLD `process_identity.rs:160`) for terminal subjects. Its retention follows the subject lifetime and its bound aliases, not only the last displayed pid. The map must be bounded by the retained process/reuse evidence. A source switch is not a terminal event.

##### Asynchronous observation states

Main's synchronous `scan` at `CT/census/scan.rs:610` has none of these worker states. The rebuild adds them in that file. The sources are OLD `PublicationStage:701`, `PublicationWorker:708`, `StageAnswer:714`, `CaptureFreshness:772`, `PublicationEvidence:823`, `AttributedInvocations:923`, `DetailRead:989`, and `ScanHistory:1618`.

| State | Rust holder to add | Transition and trigger | Impossible state |
|---|---|---|---|
| Worker Available | `PublicationWorker::Available` | One request is accepted; state becomes Running. | No second queued request can build up behind one blocked operation. |
| Worker Running | `PublicationWorker::Running` | Its matching reply completes; state becomes Available. | Publication does not wait for this worker. |
| Answer Pending; answer Completed | `StageAnswer::{Pending, Completed(Answer)}` | Polling receives a complete reply. Only Completed replaces the retained answer. | An incomplete answer cannot replace a published observation. |
| Queued detail | `DetailRead::Queued` | A new lifetime awaits dispatch. Dispatch changes it to Reading. | An unstarted detail has no claimed result. |
| Reading detail | `DetailRead::Reading` | A matching reply changes it to Ready. A source change changes it to ReadingChanged. | A stale source reply cannot become current detail. |
| Ready detail | `DetailRead::Ready(Arc<T>)` | Refresh dispatch changes it to Refreshing. A source change changes it to SourceChanged. | Ready has a completed payload. |
| Refreshing detail | `DetailRead::Refreshing(Arc<T>)` | A reply replaces the payload and returns to Ready. A source change changes it to RefreshingChanged. | A pending refresh cannot erase the last completed payload. |
| ReadingChanged; RefreshingChanged | `DetailRead::{ReadingChanged, RefreshingChanged(Arc<T>)}` | The old in-flight reply completes. The source remains changed and its new read gets priority. | Completion for an old source does not confirm the new source. |
| SourceChanged detail | `DetailRead::SourceChanged(Arc<T>)` | Priority dispatch changes it to Refreshing; the retained payload remains identified as old-source data until replacement. | Retained data is not evidence that the changed source was just read. |
| Current capture; retained capture | `CaptureFreshness::{Current, Retained}` | A completed capture discovery supplies Current for its observation; reuse is Retained. | Retained registration absence cannot be treated as a new registration-loss proof. |
| Current publication; selected current process evidence with named capture freshness | `PublicationEvidence::{Current, Selected { processes, captures }}` | Each publication records which inputs completed. | A partial read cannot silently claim full observation coverage. |
| Completed attribution | `AttributedInvocations { identities, measurements }` | One attribution reply replaces the retained result. `for_census` reuses only entries whose invocation identity still matches. | CPU from a reused pid cannot be assigned to its replacement invocation. The CPU scan stamp stays with the measurement payload. |

`DetailDiscovery` (ADD, OLD line 1019) owns new-lifetime, source-change and refresh work. Dispatch new lifetimes first, source changes next, ordinary refreshes last. A source change during a read is a distinct state; it must not wait as an ordinary refresh. Keep the old read-order and changed-source regression tests. Do not add speculative CPU work to admission.

##### Invocation lifetime states

ADD one `Invocations` owner map (OLD `CT/census/scan.rs:1952`), holding one `InvocationRecord` (OLD line 1821) per canonical execution. `Membership` (OLD line 1636) is Lead or Member with a resolved lead. It is separate from lifetime; a member does not request an independent family cell.

| Lifetime state | Rust holder to add | Transition and trigger | Impossible state |
|---|---|---|---|
| Unseen | No entry in `Invocations.records` | A resolved current observation creates Live. | There is no second unseen-record store. |
| Live | `InvocationState::Live { row }` (OLD line 1800) | A matching observation replaces values in place. Lack of sufficient current row evidence changes it to Missing. A resolved end proof changes it to Ended. | A source or representative change cannot by itself create a second record. |
| Missing | `InvocationState::Missing { row, since }` | A matching live observation restores Live. A valid end proof changes it to Ended. Otherwise it keeps its last row. | One missing read does not become an end by roster omission. Missing cannot grant a first admission. |
| Ended | `InvocationState::Ended(EndedInvocation)`; `EndedInvocation` (OLD line 1715) holds immutable values, end publication time and presentation receipt | A proof consumes the active execution state. The record remains until its required publication and receipt rules permit removal. | It cannot refresh Live or restart entry. Late scans cannot append live rows into its frozen row set. |

End construction takes `ResolvedExecutionEnd` (ADD, OLD line 2462) and `ExecutionEndProof` (OLD line 2456). The proof variants are Native(`ProcessLifetimeSubject`), Registration(`RunId`) and NotSeen. `ProcessLifetimeSubject` (OLD line 2418) owns its pid, lifetime evidence and that pid's kernel observation. It prevents a shim birth from being compared with a cargo pid or a forwarded owner's birth with another representative pid.

Native end evidence includes a proved replacement, disappearance or zombie state for that subject. A current registration loss can end a registration-only `CargoReadiness::AwaitingProcess`; it cannot end a verified executor merely because its shim registration is absent. Alive evidence prevents the missing timeout. An inconclusive missing execution can reach NotSeen at `MISSING_SCAN_LIMIT`; the count must have a stated fresh-evidence clock. The old implementation used scan-number distance. The question about retained asynchronous scans is listed at the end.

##### Publication, admission and presentation

ADD `PublishedInvocations` with private groups and one state for every retained record (OLD `scan.rs:2320`). It publishes a projection, not a second owning record map.

| State | Rust holder to add | Transition and trigger | Impossible state |
|---|---|---|---|
| Live publication | `PublishedInvocation::Live(InvocationPresentation)` (OLD line 2245) | A resolved current live row with native cargo readiness is published. It can grant `AdmissibleInvocation`. | No arbitrary `CargoGroup` grants first admission. |
| Awaiting cargo | `CargoReadiness::AwaitingProcess` and `PublishedInvocation::AwaitingCargo` (OLD lines 1932,2245) | Registration proves the job but not its cargo executor. A verified cargo observation changes readiness to ObservedProcess and publication to Live. | Registration-only data cannot open a cell that claims the shim pid is the executor. It can be Listed. |
| Observed cargo | `CargoReadiness::ObservedProcess` | Native cargo proof is established; later source changes retain the resolved execution. | CPU availability is not part of this proof. |
| Missing publication | `PublishedInvocation::Missing` | The invocation record is Missing. | It cannot prepare a first cell. An existing cell is not ended by this publication alone. |
| Ended publication | `PublishedInvocation::Ended` | The record has consumed an end proof. | Omission is not an alternative end path in the roster. |
| Joined publication | `PublishedInvocation::Joined { into }` | A proved binding transfer removes the old independent owner and names the canonical destination. | A join cannot leave the old owner fading as a second execution. |
| NotPresented | `Presentation::NotPresented` (ADD, OLD line 1655) | An actual visible row span changes it to Presented. An unpainted end can retire after the required end publication. | Allocation alone cannot claim paint. |
| Presented | `Presentation::Presented { spans, departure }` | Current per-process spans update the receipt. Completion changes it to Departed. Hiding changes it to Withdrawn and completes the receipt. | The lead's receipt cannot substitute for a child's actual painted span. |
| Withdrawn | `Presentation::Withdrawn` | Explicit hiding removes paint ownership and completes departure obligations. A live command shown again can receive a new presented span. | A hidden ended record cannot remain forever waiting for invisible paint. |
| Departed | `Presentation::Departed` | The actual departure receipt completed. The census can remove the ended record. | A later old publication cannot present the ended invocation again. |

`InvocationPresentation` (ADD, OLD line 1668) shares only the receipt state and resolved id. It is not a second location map. ADD `AdmissibleInvocation` (OLD line 2268), an `AdmissionEvidence` implementation. At the crate boundary it becomes `AdmissibleCell` (OLD `TP/draw.rs:1489`). `DrawableCell::prepare` requires it. `TileCells::admission` has no default that manufactures a proof.

##### (a) Summary cut and visible waiting commands

Section (e) replaces the earlier Phase 24 rule that also listed Celled commands. The summary now contains only Listed invocations and departures whose one site is the summary. It never adds Celled rows merely because free space exists.

ADD `SummaryDemand` in `TP/grid.rs` (OLD line 122). It holds a nonzero required minimum and the Listed count. Change `TileDemands.summary` from main's `usize` (`TP/grid.rs:110`) to this type and update all callers. ADD `ListedCommands` (OLD line 147) as an ordered derived view. It has no independent mutable membership.

The required summary height is table overhead plus one row per Listed command. Keep oldest-first order. All published visible live invocations without cells must be in that set. Optional summary growth uses only free rows after cell minima fit. A foreign member added to an already Celled family cannot increase required summary demand.

A successful admission changes Listed to Celled in the same committed frame. The summary removes that row before paint. Proven capacity loss changes Celled to Listed, newest cells first. No intermediate frame may place the invocation in neither site or both sites. A summary-only viewport too small for all mandatory rows needs the explicit policy in question 3; a count row is not already approved.

##### (b) First paint, entry ink and ended departure

Main has no invocation entry/departure clock. ADD these types in `TP/draw.rs` and `TP/grid.rs`, using the OLD sources below.

| State | Rust holder to add | Transition and trigger | Impossible state |
|---|---|---|---|
| Waiting | `CellLifecycle::Waiting` (OLD draw line 785); `AllocatedArrival` (OLD grid line 481) holds no clock | An admitted, whole body is actually painted. `FirstPaint` starts Entering on that frame. | Allocation or time spent waiting cannot advance entry ink. |
| Entering | `CellLifecycle::Entering(CellEntryClock)`; `CellEntryTimeline` (OLD draw line 703), `CellEntryClock` (line 744) | First paint starts one ease-out clock for text and geometry. Completion becomes Resident. An end consumes it and becomes Leaving. | Entry ink cannot keep increasing after the end transition. |
| Resident | `CellLifecycle::Resident` | A layout change moves the existing cell. A proved end becomes Leaving. | A header or source change cannot create another entry. |
| Leaving | `CellLifecycle::Leaving(EndedCell)`; `EndedCell` (OLD draw line 807) | `EndedCellDeparture` supplies the end time, timing and receipt. Ease-in departure completes its receipt; the cell is then removed. | Repeated end publications cannot restart the fade. Departure completion cannot create a new summary row. |
| Immediate departure | `CellDeparture::Immediate` (OLD draw line 152) | The configured departure has no timed fade. Complete once. | It does not create an entry clock. |
| Timed departure | `CellDeparture::Fading { started, timing }` | Each frame samples the same configured clock, including after a form change. | A geometry retarget does not replace its start time. |
| First visible ink | `VisibleInk` (OLD draw line 735), built from `FirstPaint` (OLD grid line 465) | The first real body paint supplies a bounded visible ink value. Entry follows its ease-out curve. | A painted entry cannot have the fully invisible start value. |
| Live continuity; layout-managed continuity; ended continuity | `CellContinuity::{Running, LayoutManaged, FrozenAfterEnd}` (OLD draw line 798) | Layout policy selects live composition. A proved end freezes its row values and departure state. | Moving the grid cannot turn FrozenAfterEnd back into Running. |

For an end during entry, retain the last painted ink; consume the entry clock before sampling the next frame. Record the end publication time in `CARGO_TILE_FRAME_LOG` and measure the no-rise rule from that time, as already approved.

An ended cargo-tile cell retains immutable row values (`EndedCellContent`, ADD `CT/render/invocation_frame.rs`, OLD line 285). It does not retain a command-form-specific raster. Generic callers can use `CellContentRetention::{PaintedImage, RowValues}` and `RetainedCellBody` (OLD draw lines 844,854); cargo-tile selects RowValues. The current command form composes both live and ended values.

A child's receipt covers its own visible process-row span. Its end does not end its live parent. If lead and child end in the same publication, first apply that publication to all owned rows, then freeze the ended family. ADD `GroupProcessRows::{Live(Vec<_>), Ended(Box<[_]>)}` in `CT/roster.rs` (OLD line 240). There is no transition from Ended back to Live. This excludes the stale-live child in a frozen ended family.

Do not remove a presented ended group because a later scan omits it. Retain it until its receipt completes or an explicit withdrawal applies. Keep the configured fade clock; capacity handling and hiding must name any permitted early withdrawal. Whole-cell shrink geometry is Phase 31's task. Phase 28 does not claim that its types already prevent the headless shrinking departure.

##### (c) Required minima, capacity and frame preparation

ADD the following proof types in `TP/grid.rs`, sourced from OLD `RequiredMinima:372`, `InsufficientViewport:360`, `SummarizedCommands:441`, `WholeCellDrawing:594`, `ViewportDrawing:1000`, and `FramePlacement:157`.

| State or proof | Holder to add | Transition and trigger | Impossible state |
|---|---|---|---|
| Required body | `CellPackingRows::Required` | Current first drawable body, pid row, foot and border establish the cell minimum, with the configured readable floor. | An optional previously drawn prefix cannot become a required minimum. |
| Optional painted body or prefix | `CellPackingRows::{Painted, Prefix}` | A fitted frame can use available rows above its minimum. | Optional growth cannot prove that a live cell must be evicted. |
| All minima fit | `RequiredMinima::check` returns `WholeCellDrawing` through the checked capacity path | Try the allowed column counts and ordered packings with the current required summary and cell minima. | A search failure for one motion candidate is not a capacity-loss proof. |
| Insufficient viewport | `InsufficientViewport`, private constructor from the complete minimum check | No allowed ordered packing holds all minima. Its method may build nonempty `SummarizedCommands`. | A normal relayout or optional row increase cannot move a live cell to summary without this proof. |
| Checked viewport | `ViewportDrawing` | Only a checked whole-cell outcome and a proved summarized set construct the frame. | A painter cannot bypass the whole-body and viewport bounds checks. |
| Candidate membership | `PlacementTrial` (ADD `CT/render/invocation_frame.rs`, OLD line 148) | A candidate checked drawing derives Listed and cell assignments for measurement. | It has no paint capability. |
| Committed membership | `FrameInvocationPlacement` | Final checked membership is committed once, after the policy is frozen and final measurement completes. | Old frame placement and a new candidate cannot both authorize paint. |

Keep the exhaustive small ordered-packing test required by Phase 24. Greedy ordered fill must agree with exhaustive search for the stated small cases. A Result error from a heuristic is not `InsufficientViewport`.

Current main's frame hooks are at `TP/draw.rs:171` (`TileCells`) and `CT/render.rs:463` (`prepare_frame`). ADD the OLD bounded preparation protocol:

- `FramePreparation::{Unprepared, Allocated, Committed}` in `CT/render.rs` (OLD line 418) distinguishes preparation from final ownership.
- `CommandFrameLayout::{Unprepared, Prepared, Committed}` (OLD line 4162) owns the command form. A `CommandHeaderForm` is one struct over its columns (OLD line 4152). Do not restore redundant variants that also encode the parent-column fact.
- `FrameLayoutPreparation::{Ready, MeasureSummaryAgain, MeasureAgain}` in `TP/draw.rs` (OLD line 1588) states what changed. Ready permits the measured result. Summary-only changes invalidate summary readings and candidate-summary caches. A command form or command row policy change invalidates command readings too.
- `summary_demand_for(placement, width)` and `summary_rows_for(placement, area)` are pure candidate queries. They do not replace trial or committed membership.
- At most two `prepare_painted_frame` trials select policy. `freeze_painted_frame` then freezes form, row ends, gap and whole-command policy. If required, perform a final measurement under that frozen policy. Do not prepare again after that measurement. `frame_placement` commits exact final membership; it does not change policy.

Use current canonical widths and the current readable-height proof for the initial preparation. Do not use the last painted height as the next frame's policy input. Optional rows are shared only after mandatory allocations fit, and only to cells whose current measurements can use them.

Keep Phase 26's fresh resolve calls: current `TP/grid.rs:1608` (`measure_moving_height_rows`) and line 1713 (`select_moving_heights`) each call `drawing_at_with_height_snap` at line 3097. It resolves with that pass's current row measurements. The saved patch's pre-measurement destination reuse is excluded. `drawing_from_destination` at line 3112 and `distinct_motion_steps_for_test` at line 3045 stay for the landed sampling behavior.

ADD live-only destinations (`LiveDestination`, OLD grid line 2311; `LiveStep`, line 2372). Retargeting replaces the live destination. Ended departure geometry and clocks stay outside this queue. A queued destination cannot restore a departing cell as live. This does not authorize a new motion scheduler architecture.

##### Closure 18 type rules carried forward

| Finding | Rebuild contract and source | Excluded state |
|---|---|---|
| F030 | `Invocations` publishes Joined explicitly; `Roster::observe_invocations` projects it. Main's `Roster::observe` (`CT/roster.rs:429`) is replaced. | An omitted forwarding alias cannot create a separate fading job. |
| F045, ended child | `PaintedProcessRowSpan` carries resolved owner, pid, site and visible span; receipts come from the assigned child's spans. | A lead receipt cannot retire its child before that child's departure completes. |
| F049 | `Presentation` has NotPresented, Presented, Withdrawn and Departed; hiding consumes visible allocations and completes their receipts. | A hidden ended invocation cannot wait for a paint that cannot occur. |
| F050 | Lifetime joins require their subject proof; terminal bindings survive record retirement. Same-scan live capture binding is the separate valid proof described above. | An unread source cannot revive a terminal identity or merge real native reuse. |
| F051 | Row layout supplies keyed spans. Ended-row selection consumes those spans. | A pid in an argument cannot be parsed as ownership evidence. |
| F052 | `AdmissibleInvocation` implements `AdmissionEvidence`; `AdmissibleCell` crosses the trait boundary. | A missing, ended or registration-only row cannot prepare its first cell through a default proof. |

##### (e) One invocation, one place on current main

##### e1. One committed placement per resolved invocation

Main has no `InvocationPlacement` or `CT/render/invocation_frame.rs`. ADD that module, using OLD lines 21-158 as a starting point. Its private placement map covers every visible canonical invocation, including each family member. A Missing publication retains an existing site while its end remains unproved; it does not grant a new cell. An AwaitingCargo publication remains Listed until its native executor proof exists.

| Placement state | Holder to add | Transition and trigger | Impossible state |
|---|---|---|---|
| Listed | `InvocationPlacement::Listed` | Initial visible live publication, including AwaitingCargo. Successful checked admission changes it to Celled in that frame. | A Listed invocation cannot also authorize a cell row. |
| Celled | `InvocationPlacement::Celled(InvocationCellId)` | One checked cell assignment. A family member names its lead's cell. Proved capacity loss changes it to Listed; an end changes it to Departing at this cell. | One invocation cannot name two cells. An independent family member is not silently swallowed by another lead's child projection. |
| Departing in summary | `InvocationPlacement::Departing(DepartureSite::Summary)` | A presented Listed row ends. Its receipt completes, then it becomes UnshownAfterDeparture. | A summary departure cannot also own a cell body. |
| Departing in cell | `InvocationPlacement::Departing(DepartureSite::Cell(InvocationCellId))` | A presented cell row ends. Its existing site and receipt persist through its fade. Completion changes it to UnshownAfterDeparture. | Losing the grid slot after completion cannot send the row back to the summary. |
| Unshown after departure | `InvocationPlacement::UnshownAfterDeparture` | A completed departure has no allocation; the census can retire it. | This state cannot fall back to Listed or summary departure. |
| Hidden or never presented end | No paint assignment; `Presentation::Withdrawn` or `NotPresented` supplies the reason | Hide consumes assignments. A never-presented end satisfies its end-publication rule without creating a painted departure. | No one-variant reason enum or extra location store is needed. |

`DepartureSite` and `InvocationCellId` are ADD types from OLD `invocation_frame.rs:25,21`. The departed site must come from the invocation's receipt and prior actual assignment, not from the absence of a cell in the new grid result. OLD `FrameInvocationPlacement::checked` at line 65 used a summary fallback for an ended id without a cell; the Phase 25 outcome shows why that rule must change.

The grid's generic `FramePlacement` is an immutable outcome/view used to derive this commitment. It is not a second mutable owner. `ListedCommands` is a derived ordered view. `PlacementTrial` is separate and cannot paint. Compute a checked result once and clone an immutable value only when two measurement consumers need it; do not solve the same input twice.

Summary plans borrow only Listed and summary-site Departing entries. `CellAssignment` (ADD, OLD module line 134) grants one cell's row plan. Cell plans borrow only that assignment. No painter falls back to the full roster because a placement is unavailable. `Presentation` remains a receipt, not a second placement owner.

##### e2. Parent, shim and executor resolve before admission

Use the binding states in the base model. `InvocationHandover`, `ForwardingEdge` (OLD scan line 2882), `LifetimeMatch` and `LiveCaptureBinding` carry the applicable evidence into resolution. An alias or metadata-source change updates one canonical record. It does not publish an end, start entry or create a new cell id.

A positive independent branch or a different native lifetime can create a distinct owner. A missing read cannot. Live same-scan co-presence plus the verified capture-parent link can transfer the process owner to Captured, even with Unavailable cargo lifetime. No overlap means no such proof. Retained parent aliases must keep their terminal relation after their cell receipt completes.

##### e3. One owner and one paint allocation per process row

Main's `TrackedGroup.rest` (`CT/roster.rs:169`) is an unkeyed mutable row Vec. ADD these ownership types:

- `InvocationProcessRowKey = (ResolvedInvocationId, Pid)`; census source OLD scan line 1833, renderer source OLD `invocation_frame.rs:152`. Matching lifetime evidence is part of the row value, not replaced by a pid-only key.
- `InvocationFamilyRows` (ADD scan, OLD line 1844) projects canonical record rows once. Repeated publication replaces values for the same key; it does not append.
- `OwnedRosterGroups` (ADD roster, OLD line 623) is the private constructor for the family projection. It assigns each key to one family across live and ended groups. Retained ended ownership takes precedence, then a real live lead, then child projection. It must not remove a live lead's rows because another family also names it as a child.
- `InvocationProcessRows` and `CellAssignment` (ADD renderer module, OLD lines 158,134) derive `CellProcessRows` (OLD line 274) from committed placement. Stable order is a view of these keyed rows.
- `PaintedProcessRowSpan` (ADD renderer module, OLD line 316) names key, site and actual visible rectangle. Each process row has one contiguous allocation; wrapped command lines are in that allocation. Receipts use this cell's spans, never a shared unqualified span Vec.

When a lead and child end together, update both from that publication before freezing `GroupProcessRows::Ended`. Later family projections cannot create a second owner for those same resolved row keys while the departure is retained. A genuinely reused pid has a distinct proved execution identity and remains distinct. Do not merge such rows in the roster. Do not add a draw-time set that skips duplicate text.

##### e4. One current command form, including ended cells

Main already separates `FrameCommandColumnGrid` and `FrameSummaryColumnGrid` (`CT/render.rs:2884,2887`). Keep that separation. Add the sole `CommandHeaderForm` and frozen `CommandFrameLayout` described in part (c).

Choose the command form from the narrowest final command allocation and the current command row set. Every command cell uses that same field set; wider cells can give the fields more width. The summary selects its own form. A form change recomposes the existing cell assignments, including immutable ended row values. It does not allocate another body, change identity, restart entry or reset an end clock.

The column Vec is the one form representation. Do not also store a variant whose name separately claims whether parent is present. No body paint method accepts an independent field set after the frame form is committed.

##### e5. CPU measurement does not gate admission

Keep the current accounting states. They already express the needed distinctions; do not add another Awaiting/Sampled flag to the lifecycle.

| State | Current Rust owner | Transition and trigger | Impossible state |
|---|---|---|---|
| Reading, including zero | `Measurement::Reading(T)`, `CT/census/invocation_cpu_accounting.rs:86` | Established evidence produces a reading. A later valid sample updates it. | Zero cannot be confused with absence. |
| First observation | `MeasurementAbsence::FirstObservation`, line 95; `InvocationCpuBaseline::AwaitingFirstSample`, line 327 | A first counter establishes the baseline; a later compatible counter establishes a rate. | A first absence cannot be reported as measured zero. |
| Established baseline | `InvocationCpuBaseline::Established` | Compatible observations update accumulated work. A different identity starts a separate baseline. | A reused pid cannot inherit the old execution's counter. |
| Read failed; unproven; retained native rate | `MeasurementAbsence::{ReadFailed, Unproven, RetainedRate}`, line 95 | The collected evidence selects the reason. A sound later sample can supply a reading. | A failed read is not a first-sample state or a measured value. |
| No CPU publication; published CPU | `CpuPublication::{NeverPublished, Published(Instant)}`, line 223 | A due accounting publication replaces the held values and its timestamp. Before the deadline the applicable held values remain. | Sample time and publication time are not one clock. |
| First smoothing sample; observed smoothing sample | `CpuSmoothingSample::{AwaitingFirstSample, Observed(Instant)}`, line 233 | First sample uses the poll interval. Later samples use all elapsed time since the previous sample. | Late sampling cannot shorten the configured smoothing window by applying one nominal poll step. |
| Replace memory; keep memory | `MemoryReport::{Replace, Keep}`, line 243 | Reporting cadence and evidence select replacement or retention with CPU publication. | An independent frame paint does not create a new memory sample. |

`AdmissibleInvocation` requires current live cargo identity and native executor proof. It does not inspect CPU readings, compiler count or the first CPU baseline. A new cell can show unavailable CPU immediately. Keep `a_late_cpu_scan_keeps_the_configured_smoothing_window` at `CT/census/scan.rs:7381` unchanged.

##### e6. Reuse completed work with complete typed inputs

The saved patch adds width-layout reuse and borrowed row readings to the old renderer. These are rebuild additions, not current-main types. Add `TableLayoutInputs { width, columns, constraints, column_spacing }` and `SolvedTableColumns` in `CT/render.rs`. The solver must read only `TableLayoutInputs`. The key therefore cannot omit a value that the width solver reads. Equal fields share solved widths across row identities and heights. Paint color, row values and height remain inputs to their own composition stages; a shared width result does not grant paint ownership.

Keep row-end, gap and whole-short-table policy changes explicit. Reuse a motion table-height or whole-short probe only when its complete semantic input is equal. Do not retain a reading across a changed policy to satisfy a query-count test. The bounded preparation result in part (c) states the necessary invalidation.

The patch's test-only `CaptureMinimumRows::{Unmeasured, Measured { inputs, rows }}` caches repeated capacity checks. `CaptureMinimumInputs` includes viewport, session widths, complete group row values and lifetimes, and ancestry. A change to any of these requires a fresh reading. This test cache does not replace the product's capacity proof.

Keep the patch's borrowed prepared-row lookups and buffer row slices. Keep its deterministic foreign-load and one-place work caps with every behavior assertion. The saved foreign caps count census passes, draws, frames and invariant checks (40-column fixture: 10/13/13/13; 80-column fixture: 15/18/18/18). They do not replace the Phase 28 under-one-second timing requirement or the paired release CPU/frame gate. Record host and load with timings. Never infer a release CPU gain from a fixture's run time.

Do not restore the patch's pre-measurement grid destination reuse. Do not restore its `native_cpu_seconds` tolerance check. Current main's Phase 27 oracle and current CPU retention/accounting rules remain the authority for that test.

##### Phase 27 measurement stamp through asynchronous reuse

This is a required difference from `d22c672b`. Main's `CpuScanPublication` is test-only (`CT/census/cpu_observation.rs:23`). Its states are:

| State | Current holder | Transition and trigger | Excluded state |
|---|---|---|---|
| Unobserved | `CpuScanPublication::Unobserved` | A fixture or a reader without the CPU recorder supplies data. Starting the recorder allows later accounting scans to produce Observed stamps. | Ordinary product output cannot claim a recorded CPU scan. |
| Observed scan | `CpuScanPublication::Observed(u64)` | `cpu_observation::record`, line 56, finishes one scan record. Its stamp travels with those measurement values. | A new census cannot relabel retained CPU as newly measured. |
| Accepted publication | `CpuReaderObservation.accepted_publication`, line 32 | `accept`, line 123, receives the Scan chosen by the terminal loop. | A scanner result not accepted by the terminal cannot stamp a frame. |
| Drawn stamped frame | `publish_frame`, line 134, consumes the accepted stamp for output | `App::draw` calls it before backend output. No draw means no OSC stamp. | Drain time in Python cannot replace the actual frame publication clock. |

Keep every current hook: `cpu_observation::work` at `CT/census/scan.rs:1331`, before accounting consumes the contribution; `InvocationCpuAccounting::observe_cpu` at line 665 of its file; `InvocationMeasurements.cpu_scan_publication` at line 145; `Scan.cpu_scan_publication` at `CT/terminal.rs:49`; `accept` in `drain_scans` at line 292; and `publish_frame` in `CT/app.rs:314`.

ADD the stamp field to both construction paths of the rebuilt `AttributedInvocations`. Empty attribution carries Unobserved. `for_census` (OLD `scan.rs:956`) filters the old measurement maps by matching invocation identity and carries `self.measurements.cpu_scan_publication` unchanged. It must not use the new census's sequence, current time or current discovery stamp. A completed replacement attribution supplies its own stamp with its own values. No extra product publication clock is needed.

Required red-first test: `a_reused_attribution_keeps_its_measurement_stamp`. Supply an observed attribution for scan A, reuse it for a newer compatible census B, and check that the result still carries A. Also cover the empty result and replacement attribution. The test exercises `for_census`, not a constant field comparison.

Keep current `NativeCpuHistory`, `NativeCpuOracle`, `UniqueProcessContributions`, `FixtureCpuOwnership` and `stopped_compiler_cpu` (`CT/shim_registration/reader_scenario.py:955,988,777,791,726`). They use native counters, platform retention, elapsed smoothing, integer-nanosecond scan times and publication deadlines. Keep all five `reader_cpu_oracle_*` tests (`reader_scenarios.rs:108` onward), the live cache-server reader and its SIGSTOP kernel check. The current oracle is not the excluded patch tolerance check. Phase 28 does not change product accounting or the copied retention rule.

##### Main differences that govern the rebuild

This list covers state-model and rebuild paths. It does not treat unrelated changes elsewhere on main as permission to restore an old file wholesale.

| Area | Current main versus `d22c672b` | Rebuild rule |
|---|---|---|
| Invocation lifecycle | Main has scan-local groups and omission-based roster endings. The old tree has `Invocations`, explicit publications, readiness, receipts and admission proofs. | Add the explicit owner states and remove the competing omission path. |
| Process continuity | Main replaces `ProcessIdentities.present` per scan and has no binding/terminal maps. The old tree retains missing subjects and resolved binding evidence. | Rebuild the subject proofs, same-scan capture join and terminal facts; preserve real branch and reuse tests. |
| Lifetime comparison | Main's `ProcessLifetime` (`CT/birth_stamp/process_lifetime.rs:15`) has no old `compare_registration`/`verify_process` helpers. | Add the matching-subject comparison needed by binding and end proofs; preserve native birth precision. |
| Census stages | Main uses one retained full-discovery `System`, synchronous detail reads and synchronous attribution in `scan`. The old tree adds workers, freshness and retained results. | Rebuild bounded worker states and new-job/source-change priority. Compare refresh work with current main before a cost claim. Do not add full-system refreshes without measurement. |
| Roster and render ownership | Main has no `OwnedRosterGroups`, process-keyed paint module, current command form owner or exclusive placement. | Rebuild these owners; do not copy the old ended-to-summary fallback. |
| Framework API | Main uses `TileDemands.summary: usize` and no lifecycle admission/placement hooks. The old tree has typed summary demand, checked capacity and proof-carrying hooks. | Change callers, including cargo-handler, with the approved API change. No default admission proof. |
| Frame preparation | Main uses its existing prepare/measure path. The old round-3 tree adds bounded trials, frozen command policy, pure candidate summary queries and summary-only invalidation. | Rebuild those states and current-input preparation. Keep final capacity and paint under the same policy. |
| Grid resolution | Phase 26 restored fresh resolves after row measurements. The saved patch shares a destination resolved before those measurements. | Keep current fresh resolves. Reuse can return only with the specified per-motion-frame fresh-versus-reused equivalence test. |
| Motion test sampling | Main already has `drawing_from_destination`, `distinct_motion_steps_for_test`, geometry-boundary sampling and row slices from Phase 26. | Keep the landed helpers and assertions. Merge old-only tests onto them; do not restore duplicate fixture work. |
| Render work reuse | Main does not have the saved patch's exact-input width cache, borrowed lookups or old-only capture-minimum cache. | Add complete typed keys and reuse only work with equal inputs. Retain the patch's three cache regressions. |
| CPU smoothing | Main has `CpuSmoothingSample` and elapsed alpha. The old tree predates them. | Keep the separate sample clock and late-scan test. |
| CPU recorder and reused attribution | Main has the test-only Phase 27 hook chain but no `AttributedInvocations`. The old `for_census` constructs measurement structs without a stamp. | Carry the old measurement stamp through reuse; red-first test above. Keep all hooks test-only. |
| Reader startup and readiness | Main stages the same reader executable in a macOS fixture temp directory (`CT/shim_registration/reader_scenarios.rs:182`), waits for headers/native pids and requires a reporting-window sample span. The old tree predates these safeguards. | Keep these behaviors and merge remaining patch reader work onto them. Do not restore parent-column-only readiness or sample-count-only completion. |
| Reader CPU oracle and cleanup | Main has the Phase 27 oracle, scan/frame stamps, integer deadlines and SIGKILL/PTY-close-before-wait cleanup. Neither the old tree nor its saved strict tolerance patch provides this model. | Keep main's oracle and cleanup. A slow Phase 27 live CPU test is already assigned to later work; it does not justify weakening these tests. |
| Panic report test | Main accepts a `Display` backtrace and tests a known frame (`crates/tui_pane/src/runner/lifecycle.rs`). | Keep the landed behavior; do not restore native backtrace resolution in the test. |
| Consumer path | Main's cargo-handler renderer is `crates/cargo-handler/src/render/mod.rs`; the old tree used `src/render.rs`. Main also has newer render tests in that directory. | Update current callers in place; do not replace the directory with the old file. |
| Fixtures and work limits | Foreign-load and one-place fixtures are old-tree additions; their saved patch replaces wall-time assertions with bounded work. | Rebuild behavior checks and work counts. Measure each Phase 28 named slow test alone and under suite load; no fixture/assertion reduction to meet time. |
| Exports, features and records | The old lifecycle types require tile exports, `test-clock`, constants, nextest entries and changelog records not all present on main. | Merge only the required additions into current files, including all consumers. This is not a whole-tree reset to the unlanded commit. |

##### Proof obligations before landing

The implementation must give each changed transition a behavior test that is red before the repair and green after it. Use supplied process lifetimes, controlled observation freshness and injected motion time. The binding tests must reach the live Process(Unavailable)/Captured overlap and the subsequent end; two hand-built renderer groups alone do not prove census identity.

Keep tests for first-frame summary removal, one cell through forwarding/source changes, one row key per family, a lead and child ending in one publication, a full departure receipt, a header-form change at an end, and a completed departure staying absent from summary. Check actual rendered spans and ownership, not only a demand vector. Keep native reuse and independent-branch cases.

Preserve every Phase 26 and Phase 27 assertion. Preserve the Phase 24 exhaustive capacity proof test and the checked-paint constructors. No new allow/expect attributes. The Phase 28 named old-only slow tests must take less than one second alone and pass with the full suites. The paired release capture must satisfy Phase 25's unchanged landing rule; at most three repair rounds. No test or capture ran for this document-only step.

##### Open questions for the showrunner before code

1. **Completed departure site.** Approve the explicit terminal placement rule in e1: after a cell receipt completes, the ended invocation becomes UnshownAfterDeparture and cannot use OLD `checked`'s summary fallback. Also confirm which existing capacity withdrawal, if any, may complete a departure before its configured fade; keep hiding as an explicit withdrawal. This resolves the Phase 25 one-frame summary return without taking Phase 31's whole-cell shrink work.
2. **Missing clock under retained inputs.** Does `MISSING_SCAN_LIMIT` count fresh discovery observations that cover the subject, or all publication iterations as the old scan-number subtraction did? Recommended: count only qualifying fresh observations; retain Missing while its relevant input stage is pending. Native and current registration proofs can still end it immediately under their stated rules.
3. **Summary below its physical minimum.** If the viewport cannot hold header, foot and one row for every Listed command even with no cells, what is the approved visible behavior? The prior ruling requires individual rows and rejects a hidden wait count; it does not define this physically impossible packing. Recommended: name an explicit viewport-limit result and a visible overflow/navigation policy before code, without treating missing rows as Celled or completing their receipts.
4. **Remaining Phase 25 capture failures.** Confirm which of rows-cut-with-free-space and reduced admission from the 30-row foreign body must be repaired in Phase 28, and which, if any, are explicitly moved to Phase 31 by the landing capture. Recommended: keep both in Phase 28's minimum/optional-row and one-place contracts until the showrunner names a transfer. CPU/frame cost remains a Phase 28 landing gate and is not replaced by the fixture timings.

**Showrunner answers to the state model questions (2026-10-27):**
1. Completed departure: approved. After its receipt completes, the ended invocation is UnshownAfterDeparture and never takes the old summary fallback. Only an explicit withdrawal ends a departure before its fade: the user hides it, or capacity withdraws it. Each is a typed withdrawal that completes the receipt. Nothing else does.
2. Missing clock: approved as recommended. Count only fresh discovery observations that cover the subject. Missing stays while its input stage is pending. Native and registration proofs can still end it at once.
3. Summary below its physical minimum: rare; keep it small. An explicit type result, e.g. SummaryFit::Clipped { shown, omitted }. Draw the rows that fit; the last visible row is an "N more" row. No scrolling, no navigation. Omitted rows are not Celled, and their receipts do not complete. The "no hidden wait count" ruling covers the normal case, not a terminal that cannot physically hold the rows.
4. Both capture failures stay in phase 28 (rows cut while free rows exist; reduced admission from the 30-row foreign body). CPU and frame cost stay a landing gate. At most 3 repair rounds, then land only if equal to or better than main on every live count.

**Landing rule:** Phase 25's, unchanged. At most 3 repair rounds.

**Constraints from prior phases:** Phase 25's Constraints hold, with these exclusions from the rebuild sources:
- **No grid destination reuse.** Phase 26 restored the resolve calls: `measure_moving_height_rows` and `select_moving_heights` (`crates/tui_pane/src/tiles/grid.rs`) each call `drawing_at_with_height_snap`, which resolves the destination after this pass's row measurements. `on_24a_round3.patch` passes one destination resolved before the measurements into both (`drawing_from_destination(…, destination)`). That change is not restored. It can return only with an equivalence proof: a test that compares the reused destination with a fresh resolve after the measurements, on each frame of the motion fixtures, and fails when they differ. No test fails with the reuse today (`phases/26/review_findings_2.txt` F001). `drawing_from_destination` and `distinct_motion_steps_for_test` stay as Phase 26 left them, for the test sampling.
- **No strict CPU comparison from the patch.** The patch's native counter check in `reader_scenario.py` (`native_cpu_seconds` and its tolerance against the compiler pid) is not restored. The CPU check is the one Phase 27 landed (`NativeCpuHistory`, `NativeCpuOracle`, `UniqueProcessContributions`, `FixtureCpuOwnership`, `stopped_compiler_cpu` in `reader_scenario.py`); the patch's other reader script changes merge onto it.
- **Phase 27's test-only CPU recorder stays, with each hook.** Phase 27 changed no product accounting. It added `crates/cargo-tile/src/census/cpu_observation.rs` (`#[cfg(test)]`) and this chain: `cpu_observation::work(pid, identity, &work, evidence)` in `census/scan.rs`'s attribution loop, before accounting consumes the counters → `InvocationCpuAccounting::observe_cpu` (`census/invocation_cpu_accounting.rs`) → `InvocationMeasurements::cpu_scan_publication` (`CpuScanPublication::{Unobserved, Observed(u64)}`) → `Scan::cpu_scan_publication` (`terminal.rs`) → `cpu_observation::accept` in `drain_scans` → `cpu_observation::publish_frame()` in `App::draw` (`app.rs`), which writes the OSC 777 `cargo-tile-cpu` stamp. `d22c672b` builds `InvocationMeasurements` in `AttributedInvocations::for_census` (`census/scan.rs`) and can reuse an older attribution result for a newer census. The rebuild carries the stamp of the scan that measured the CPU values through that reuse, so a newly published census never stamps retained CPU values with another scan's id. Test, red first: `a_reused_attribution_keeps_its_measurement_stamp` (supplied scans: a census published on a reused attribution carries the reused result's `CpuScanPublication`).
- **Phase 26's reader safeguards stay:** the staged reader copy on macOS (`reader_scenarios.rs`), the reporting-window span in `cpu_observation_is_complete`, the readiness waits on table headers and native pids, and the elapsed-weighted CPU smoothing with its test `a_late_cpu_scan_keeps_the_configured_smoothing_window`.

No new `#[allow]` or `#[expect]`; no test takes a second.

**Acceptance gate:** as Phase 25, plus item 3's times before and after in the Outcome. Every Phase 26 and Phase 27 test passes unchanged on the landed tree, the five `reader_cpu_oracle_*` tests and `reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` included. `a_reused_attribution_keeps_its_measurement_stamp` fails before the stamp is carried through `for_census` and passes after.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/constants.rs`, `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs`, `crates/tui_pane/Cargo.toml` — the lifecycle motion and draw changes, their exports and the `test-clock` feature.
- `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/render/invocation_frame.rs`, `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/terminal.rs`, `crates/cargo-tile/src/probe.rs` — one place for each invocation, the render capacity cache and the frame cost.
- `crates/cargo-tile/src/app.rs` — keep the test-only `cpu_observation::publish_frame()` call in `App::draw`.
- `crates/cargo-tile/src/census/` — the census, process identity and refresh changes.
- `crates/cargo-tile/src/birth_stamp/process_lifetime.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/hook.rs`, `crates/cargo-tile/src/cargo-capture-shim.sh`, `crates/cargo-tile/Cargo.toml` — the lifetime proof, the valued options, the skip list and the `test-clock` feature.
- `crates/cargo-tile/src/shim_registration/` — the reader, wire, shared capture and app scenario tests, under the exclusions above.
- `crates/cargo-tile/tests/cli_lifecycle.rs`, `crates/cargo-tile/tests/shim_modes.rs`, `crates/cargo-tile/tests/support/foreign_load.rs`, `crates/cargo-tile/tests/support/one_place.rs` — the fixtures and item 3's caps.
- `crates/cargo-handler/src/render/` — the `SummaryDemand` callers and their tests (`render/mod.rs` on main; `render.rs` on `d22c672b`).
- `.config/nextest.toml` — the entries `d22c672b` added.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md` — the lines `d22c672b` added.

**Workers:** as Phase 25.

### Phase 29 — Move-only splits  · status: done

#### As-built

- `crates/tui_pane/src/tiles/grid.rs` and `tiles/draw.rs` are split by type cluster into the directories `tiles/grid/` and `tiles/draw/`. The move changed only module paths, `use` lines and visibility; no `tui_pane` public signature changed.
- `grid/` holds `mod`, `demands`, `pieces`, `rows`, `heights`, `motion`, `arrangement`, `visual`, `selection`, `tile_grid`, `layout`, `allocation` and `bands`. `draw/` holds `mod`, `contents`, `fade`, `rows`, `foot`, `measurements` and `paint`.
- Each test file sits beside its owner as `<module>_tests.rs`. Shared fixtures and the motion tests sit in `grid/tests/` and `draw/tests/`. The split kept all 264 tests (178 grid, 86 draw) with unchanged bodies and assertions.
- `TileSettings::min_tile_height` is a `NonZeroU16` (private field and default). `TileGrid::set_min_tile_height(u16)` keeps its signature and floor behavior.

**Files:**
- `crates/tui_pane/src/tiles/grid/` — arrangement, motion, allocation, `fits`, and their tests.
- `crates/tui_pane/src/tiles/draw/` — the grid's drawing and the foot readout, and their tests.
- `crates/tui_pane/src/tiles/mod.rs` — module declarations and the grid test lint expectation.
- `crates/tui_pane/src/tiles/settings.rs` — the `NonZeroU16` minimum tile height.

**Binds later work:** The minimum tile height stays a `NonZeroU16` through the allocation replacement ("Every cell moves toward one target layout"). Later splits keep tests beside their owners and shared fixtures in a test module ("The census scan file splits by type cluster").

**Gotchas:** The grid tests' `expect(clippy::expect_used)` sits on `mod grid` in `tiles/mod.rs` as `#[cfg_attr(test, expect(...))]`, not on a test module inside `grid/`.

### Phase 29a — The census scan file splits by type cluster  · status: done

#### As-built

`census/scan` is a module directory with one file for each type cluster. No function body or assertion changed in the move.

- `census.rs` holds `Census`, CPU attribution and capture annotation.
- `invocation.rs` holds `CargoProcess`, the shared helpers and `subtree_resource_totals`.
- `refresh.rs` holds the scan worker, `NextRefresh` and `FreeDiskSpace`.
- Each subject test module is declared under `#[cfg(test)]` in its production module.

**Files:**
- `crates/cargo-tile/src/census/scan/mod.rs` — module declarations and reexports.
- `crates/cargo-tile/src/census/scan/census.rs`, `census_tests.rs` — the census cluster and its tests.
- `crates/cargo-tile/src/census/scan/discovery.rs` — process discovery.
- `crates/cargo-tile/src/census/scan/invocation.rs`, `invocation_tests.rs` — the invocation cluster and its tests.
- `crates/cargo-tile/src/census/scan/observation.rs`, `observation_tests.rs` — observed process fields and their tests.
- `crates/cargo-tile/src/census/scan/refresh.rs`, `refresh_tests.rs` — the scan worker and its tests.
- `crates/cargo-tile/src/census/scan/resource_use.rs`, `resource_use_tests.rs` — resource totals and their tests.
- `crates/cargo-tile/src/census/scan/scope.rs` — the scan scope.
- `crates/cargo-tile/src/census/scan/tests.rs` — shared fixtures and the metadata exec child.

**Gotchas:** `resource_use` imports nothing from `invocation`, and `invocation` imports nothing from `census`. Shared helpers stay in `invocation.rs`.

### Phase 32m — CPU fix 1 on main  · status: done

Port of the session's CPU fix 1 (one retained System refreshed in place, Active/Idle pace, 2 s idle discovery, exec name refresh) onto main 655c7948 after the phase 28 ruling. Work order: the session's phases/32m/implementation_prompt.md.

### Phase 30m — Capture shim cost on main  · status: done
#### As-built

- The capture shim (v4) starts no process apart from cargo and `tee` for each cargo call. `cargo-tile`'s native capture entry runs from `main.rs` before argument parsing and takes the boot receipt and the birth stamp (`birth_stamp::capture_boot`, `observe_process`, `BirthStamp::registration_fields`, crate-visible). On the Mac, the added cost of a captured `cargo build` is 23 ms (256 ms before), and of `cargo --version` 11 ms (36 ms before).
- The stderr mirror is `sh -c 'printf r; exec tee'`, started by `ReadyCaptureMirror::start` in `hook.rs`. The workspace `signal-hook` dependency handles the entry's signals.
- `census/scan.rs` has one predicate for a matched dispatch and one for the `CargoTransport::NativeCapture` check.
- The install and reader tests share one native fixture (`shim_registration/native_fixture.rs`) and one prepared capture (`shared_capture.rs`). The native birth tests in `installed.rs` replace the shell conversion tests. The reader templates render the runner path and flags from `constants.rs`.

**Files:**
- `crates/cargo-tile/src/hook.rs`, `main.rs`, `cargo-capture-shim.sh`, `constants.rs` — the native entry, the mirror and the protocol words.
- `crates/cargo-tile/src/birth_stamp/` — boot and birth reads for the entry.
- `crates/cargo-tile/src/shim_registration/` — install, fixture and reader test support.
- `crates/cargo-tile/src/census/scan.rs` — the two predicates.
- `crates/cargo-tile/tests/cli_lifecycle.rs`, `tests/shim_modes.rs`; `Cargo.toml`, `Cargo.lock`, `crates/cargo-tile/Cargo.toml`, `CHANGELOG.md`.

**Binds later work:** `InvocationOwners` (`roster/invocation_owners.rs`) stays the one invocation identity owner across transport changes. `InstalledShim::repeat_in_same_shell` keeps both live log observations, and the interrupt test keeps its second interrupt after `cargo-stopping` with the `cargo-finished` assertion.

**Gotchas:** the native entry and the shim script name the entry program and its `--capture-shim` / `--capture-shim-passthrough` flags only through `constants.rs`; a test checks that the script uses the same words. The mirror signals ready before `exec tee`, so a failed `tee` leaves cargo with an unread stderr pipe (the fallback phase owns this).

**Ruled out:** a native mirror pump in this phase (`tee` stays the mirror).

### Phase 35m — Summary arguments on main  · status: done

#### As-built

- A summary command shows its arguments only when the whole command fits one line in the command column. Otherwise it shows the program, subcommand and toolchain-selector words (`cargo nextest run`) in the same word colors, and wraps only when those words do not fit. Command cells always show every argument.
- The rule holds in both argument views (`h` on or off). The user, 2026-10-29: "the arguments are an extravagance in the summary if the column is too narrow … we can drop them unless they can fit on one line - the cells have extra space to show them and we can look there when they don't fit in summary".
- `CommandText::named_words()` holds the words kept without arguments; `named()` and `CommandForm::WithoutArguments` read it. `UnchangedCommandInputs::measured_widths()` is the one width measure for `widths` and `for_row`, so retained widths and text stay reusable across resource updates.
- `nextest_header_text` strips SGR codes from nextest header lines before they are measured and counted; other escape bytes stay. A colored starting header reports testing before the first result.

**Files:**
- `crates/cargo-tile/src/census/command_text.rs` — `named_words()`, `named()`.
- `crates/cargo-tile/src/render/command_cache.rs` — `CommandForm::WithoutArguments`, `UnchangedCommandInputs::measured_widths()`.
- `crates/cargo-tile/src/render.rs` — summary wiring and the summary command tests, the `ExpandedCell` header and End tests included.
- `crates/cargo-tile/src/progress/capture_read.rs` — `nextest_header_text`, `last_test_start`.

**Ruled out:** keeping summary arguments when `h` is on (they wrap a narrow summary; the cells show them).

### Phase 36m — Directory header rule on main  · status: done

Port of the directory header rule (whole path; GUID middle cut; middle cut; last folder whole) onto main's render.rs. Work order: phases/36m/implementation_prompt.md.

### Phase 41 — The exec census test finds its program on PATH  · status: done

Showrunner request (2026-10-29): the 36m merge tree fails `census::scan::refresh_worker_tests::a_shell_execing_cargo_enters_the_retained_discoverys_census` on natedev with ENOENT, because the test copies `/bin/cat` and NixOS has no `/bin/cat`. The test finds its program on PATH, with no absolute tool path, and passes on natedev and on the Mac. Work order: phases/41/implementation_prompt.md.

### Phase 41c — The exec census tests pass on the GitHub macOS runner  · status: done

**Depends on:** Phase 41

**Goal:** Main CI is red on macOS: `a_shell_execing_cargo_enters_the_retained_discoverys_census` (`crates/cargo-tile/src/census/refresh_worker_tests.rs:267`) sees `Python`, not `cargo`. On the runner, `python3` is a Homebrew framework launcher that execs `Python.app`, so the copied fixture runs as `Python` (run 41138166229; diagnosis `phases/ci_python/answer.md`). The handler fixed its copy of this fixture in d2483f13 (`copied_agent_executable`, `same_pid_exec_launcher` in `crates/cargo-handler/src/census/scan.rs`): `/bin/cat` with codesign on macOS, `python3`'s `sys.executable` elsewhere.

**Spec:**
1. One fixture for both crates (showrunner, 2026-10-31: "use one form, no second helper if it can be shared"; user rule 2026-10-30: no duplication). A new dev-only crate `crates/census-test-support` (`publish = false`, as `cargo-berth-test-support`) owns the copied executable (named by the caller) and the platform's exec payload arguments. cargo-tile's test and cargo-handler's test both use it, and the handler's two helpers go.
2. Every assertion of both tests stays as it is.

**Tests:** both tests pass on natedev (NixOS) and on the Mac; the GitHub macOS job proves the Homebrew case.

**Files:** `crates/census-test-support/` (new), `Cargo.toml` (workspace dependency), `crates/cargo-tile/Cargo.toml`, `crates/cargo-handler/Cargo.toml`, `crates/cargo-tile/src/census/refresh_worker_tests.rs`, `crates/cargo-handler/src/census/scan.rs`, `Cargo.lock`.

**Phase 43 note:** Phase 43's tree changes this fixture another way (copied `/bin/sh` on Linux). At its merge, Phase 43 takes this phase's fixture and drops its own.

### Phase 44 — A captured command shows once, with the work under it  · status: done

**Depends on:** Phase 38

**Work Order.** User report, 2026-10-30 11:18 (natedev cargo-tile tab, installed main 378873e3). For each top-level command, the summary draws two rows with the same pid: `cargo --workspace` with cpu `0%` and mem `0.0G`, and `cargo nextest run --workspace` with its progress (`87%`) and no cpu. The cells split the same way: one tile shows `cargo --workspace 0% 0.0G`, another shows the full tree (child `cargo-nextest` 116% 1.1G, `cargo test` 99% 5.5G). That pid is the capture shim, `cargo-tile-capture --capture-shim <real cargo> …`, which stays alive as the parent of the real work. Its argv is parsed as the command (`cargo --workspace`), and its own cpu and mem are shown.

- **Goal:** the summary shows each top-level command once, with its full command text (`cargo nextest run --workspace`), and its cpu and mem are the work under it. Its own cell shows the detailed tree. This is the user's design ("the summary shows each top-level command; its own cell shows the detailed version"), and a tile shows one command once.
- **Spec:** find the cause first: where the shim pid becomes a row and a tile of its own (census classification of the `cargo-tile-capture --capture-shim` argv; roster and summary rows). Fix it by type: the shim process is the captured invocation's identity (one invocation, one row), its command text comes from the real cargo argv it runs, and its cpu and mem come from the invocation's subtree. No placeholder value; no procedural filter.
- **Acceptance:** a red-first test built from the real argv shape above (`cargo-tile-capture --capture-shim <path>/cargo nextest run …` with a busy child) shows one summary row with the full command and the child's cpu and mem; main fails it; each test under 1 s. Live check vs main on natedev under a real `cargo nextest run`: one row per command, the cpu column not 0% while the child works.
- **Files:** decided by the cause; expected `crates/cargo-tile/src/census/scan.rs` and the summary row code in `crates/cargo-tile/src/render.rs` / `roster.rs`. Not `crates/tui_pane/src/tiles/draw.rs` or `tiles/grid.rs` (handler unit).

### Phase 44b — A captured command keeps one row from its start  · status: done

**Depends on:** Phase 44

**Work Order.** Phase 44 design check, 2026-10-31 (`phases/44/live/design_check.txt`, shot `phases/44/live/shot44.early.png`; build 4e99762d). About 1.8 s after a captured `cargo nextest run` starts, its row for the shim pid freezes and goes grey, and a new live row with the same pid and the same text appears. The 3 s departure fade keeps both on screen. The cell does the same: a grey departing cell and a live cell, both for that pid. The row identity changes when the capture shim hands over to the real cargo.

A second cold job showed no grey copy, but for 0.27 s its row showed the child pid 1501410 instead of the shim pid. Main is worse at a captured start: a bare shim row plus the real row, two cells, for the whole job (`shot44.main_early.png`). Both are the same fault: the identity is chosen by which side of the hand-over the scan saw first.

The same job showed the branch's dur about 6 s less than main's: the branch counts from the real cargo's start, main from the shim's. The user started the command when the shim started, so the row's start and dur come from the invocation's first process (the shim). A test checks it.

**Measured with the wrong shim.** Until 2026-10-31 12:55 EDT every natedev toolchain had shim v4 with its native runner (`cargo-tile-capture`), from the dropped Phase 28 tree; main installs v3. The design check above ran with that runner in front of cargo. Re-measure the hand-over with main's v3 shim first; the fix targets what v3 shows.

- **Goal:** a captured command has one row and one cell from its first frame to its end, in the summary and in the grid.
- **Spec:** find what in the row key changes at the hand-over (for example the identity derived from the shim argv before the real cargo argv or its capture record is seen, then from them after). Fix it by type: the invocation's identity is one value that is fixed at its first sighting and does not depend on which side of the hand-over the scan saw first. No procedural filter that hides the departing copy.
- **Acceptance:** a red-first test that takes two scans of one shim pid, before and after the hand-over, and shows one row and no departure; main and 4e99762d fail it; each test under 1 s. Live check on natedev under a real `cargo nextest run` from a cold build: no grey copy of the row or the cell in the first 10 s.
**Process identity model** (written before any 44b code; 44b and 43a share it):

Model for 44b and 43a; no product change. Source: `6cadd2bd901e5d24ca6198ccc6e84ff04331bb60`. References below are under `crates/cargo-tile/src/`.

**Measured v3 fault (2026-10-31, `phases/44b/`).** At 393×89, cold nextest produced two summary rows for shim **479130** from +1.084 to +3.636 s. The old row froze at `00:01`, blank CPU, `0.0G`, and faded; the live row advanced to `00:09` by +9.979 s with both resources blank. Child cargo test **486265** reached **78%**, `0.0G` at that frame. Thus the fault also exists with main’s v3 shell shim. Full per-frame evidence: `live_v3_captured/first_10s_rows.jsonl`; key qualifications and all row ranges: `measurement_summary.md`.

**Owner and key.** The invocation’s origin owns the row: the shell shim for captured v3, otherwise the first real command process. Keep its PID as the displayed PID. A forwarding launcher, exec replacement, or real cargo executor is an execution role of that origin, not another invocation. An application boundary or independent nested cargo starts a distinct invocation, shown under its parent. One continuity owner allocates one private, immutable `InvocationKey` for the origin at first sighting. Index boot-qualified process lifetime and verified capture generation/root as aliases to that owner; neither is a replacement row key. If birth is unavailable, retain the continuous-observation identity; later proof enriches evidence without changing the key. PID reuse after a proved lifetime change gets a new key. The same key selects the summary row and its detailed cell.

**Lifecycle and fields.** `InvocationState` has explicit `Starting`, `Running`, `Draining`, and `Finished` payloads. First origin sighting starts the row; a proved forwarding/dispatch chain supplies its executor. Executor exit with capture plumbing still present enters Draining. End of all execution evidence enters Finished once and starts one fade. No transition changes key or start. Start is the origin’s kernel start; duration advances from it until finish, then freezes. An unavailable origin time has a named visible state; registration publication is not silently substituted. Live CPU and memory cover the assigned invocation work, including its helper processes. Independent child buckets remain disjoint; the parent summary uses their subtree total once. Draining retains the last valid work reading as retained; Finished keeps that final snapshot.

**Type guarantee.** The continuity owner alone constructs keys and owns lifecycle/resources; aliases store references, not copied state. Roster updates that owner’s row instead of creating a new row for another executor. `LiveResources` names `Reading`, `Sampling`, or `Unavailable(reason)`; render keeps main's rule that `Sampling` and `Unavailable` draw no placeholder (no new column words in 44b). With one continuity owner, a live row takes its executor's reading, so a live invocation whose executor has a reading never shows blank. Start time comes only from the origin: its process, or the registration the v3 shim writes at its own start, typed by source; `Unavailable` only when neither gives one (director rulings 2026-10-31). `Finished` owns the retained snapshot and fade time. These are proposed guarantees: current `InvocationResourceUse` (`census/scan.rs:134`) and `Measurement` (`census/invocation_cpu_accounting.rs:86`) permit unavailable readings; `render.rs:5065` and :5073 turn them into blanks.

**Current boundaries to replace.** `InvocationId::{Process,Captured}` (`census/process_identity.rs:77`) allows process/capture identity replacement. `CargoTransport` and `cargo_split` (`census/command_text.rs:591`, :616) need one origin/forwarding classification for v3 and v4. `Census::prepare` (:1250), `cargo_process_role` (:1287), `collapse_shims` (:2032), `direct_capture` (:2486), `identify_captures` (:2524), and `groups` (:2672) in `census/scan.rs` must consume that continuity. `row` (:3154), `registration_timing` (:3252), and `registration_row` (:3279) must use its timing/resources. `TrackedRow::refresh` (`roster.rs:146`), `RowLifetime` (:37), and `Roster::observe` (:480) must retain it through executor changes. 43a reuses this path; it adds no second identity or launcher filter.

**Phase 43 verdict.** Job a ended at 8.708 s. At 9–11 s base retained **833543**, `00:04`, **61%**, `0.2G`; cut showed **833530**, same duration/memory, **blank CPU**. Its executable role is unproved; it is not recorded launcher **832776**. Base’s extra launcher 832776 beside child 833543 at 5 s is wrong if both forward one invocation. Keep a second row only for a proved independent invocation, not to reproduce base’s defect.

- **Files:** decided by the cause; expected `crates/cargo-tile/src/census/scan.rs`, `census/command_text.rs`, `roster.rs`. Not `crates/tui_pane/src/tiles/` (handler unit).

### Phase 43a — The reader uses less CPU, on one process identity  · status: done

#### As-built

- `LinuxProcessStat` supplies borrowed stat fields for process birth, discovery and CPU accounting without allocating a field table.
- Each fresh Linux scan reads candidate details again and checks process birth and executable evidence around the reads.
- `ScanInput` admits fresh discovery; `PreparedCpuTopology` retains one scan’s ancestry and CPU read order.
- `PreparedTables` reuses tables while roster revision, hidden commands, command view, height and theme stay unchanged.
- Command preparation retains widths and wrapped text across resource updates; invocation continuity shares snapshots through `Rc`.
- Validated paint patches replay cell writes while preserving command visibility, colors, fades, rows and retained CPU readings.

**Files:**
- `crates/cargo-tile/src/linux_process_stat.rs` — the shared Linux stat parser.
- `crates/cargo-tile/src/census/scan.rs`, `crates/cargo-tile/src/census/scan_linux_details.rs` — discovery, fresh details and CPU topology.
- `crates/cargo-tile/src/birth_stamp/linux.rs`, `crates/cargo-tile/src/birth_stamp/process_lifetime.rs` — shared parsing and precise lifetime evidence.
- `crates/cargo-tile/src/render/frame_preparation.rs` — preparation inputs, retained tables and row measurements.
- `crates/cargo-tile/src/render/command_cache.rs`, `crates/cargo-tile/src/render/cell_paint.rs` — command preparation and validated paint replay.
- `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/roster/invocation_continuity.rs` — preparation ownership, revisions and shared invocation snapshots.

**Binds later work:** “Census CPU falls toward its goal” reuses the parser and fresh detail reads. “Every cell moves toward one target layout” preserves table reuse.

**Gotchas:** New drawn state joins the relevant preparation inputs. Clocks and deadlines stay live. Retained tables do not replace grid geometry preparation.

**Ruled out:** Process details retained across scans; a second launcher filter.

### Phase 45 — A new cell's text brightens from 75% to its final color  · status: merged into Phase 42

User, 2026-10-30: this work goes in Phase 42, and Phase 42 does not hold full ink. Its Work Order is now in Phase 42.

### Phase 43b — Census CPU falls toward its goal  · status: done

#### As-built

- Each scan prepares one `Census` and one CPU topology. On the Mac, the scan picks the processes that need a native birth read from that census, and `InvocationLifetimeReads::apply(self, census, observations, discovery, history)` patches their CPU and lifetimes on it; lifetimes return to `discovery.baselines` after the groups form. `RefreshHistory::measure` reads one process's CPU.
- `Census::take` reserves its maps from the process count (`HashMap::with_capacity`). Linux discovery reuses its read buffer.
- Mac census CPU: 7.248 → 4.517 CPU-s/min (median paired cut 42.2 %, spread 7.8 points, 3 pairs). Linux: 5.608 → 5.155 CPU-s/min, a cut under the spread, so no Linux gain is claimed. Both machines stay above the 0.60 CPU-s/min goal.

**Files:**
- `crates/cargo-tile/src/census/scan.rs` — census preparation, `InvocationLifetimeReads`, and the test counters `CENSUS_PREPARATIONS` and `TOPOLOGY_PREPARATIONS` read by `one_scan_prepares_one_census_and_one_topology`.

**Binds later work:** each scan prepares one `Census` and one CPU topology; Mac lifetime evidence reaches that census only through `InvocationLifetimeReads::apply`.

**Gotchas:** on the Mac the sysinfo refresh is 75.5 % of census CPU, and its executable fallback (`proc_pidpath`) is 4.98 %. Removing the executable refresh alone breaks fresh exec detection.

**Ruled out:** a direct unsafe FFI `proc_name` reader — no new `unsafe` or `#[allow]`; a safe crate read is the route.

### Phase 43c — No cargo-tile test takes a second or more  · status: parked (director, 2026-11-07: not landed at the 3-round repair cap; the head wins on natedev, 4 and 1 slow tests against 6, and loses on the Mac, 44 and 47 against 15 and 8; the work continues in Phase 43e)

**Depends on:** none. Its `census/scan.rs` test edits serialize with Phases 43b and 49; the director orders those edits.

#### Work Order

Showrunner, 2026-11-02 (rule: no multi-second tests). Ten Mac cargo-tile tests take 1.03 s to 2.90 s: in `shim_registration::reader_scenarios`, `shim_registration::installed`, and `progress::capture::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`. Names and times: handler Phase 54 Mac log. natedev also shows `reader_scenarios` tests at 1.2 to 2.3 s (2026-11-02 runs).

**Goal:** every cargo-tile test runs under 1 s on the Mac and on natedev.

**Spec:** measure each test at 1 s or more on both machines, then remove the cost that the test does not need. A floor that is product behavior is measured and reported, and adds no product surface.

**Files:**
- `crates/cargo-tile/src/shim_registration/`
- `crates/cargo-tile/src/progress/capture.rs`
- `crates/cargo-tile/src/render.rs`
- `crates/cargo-tile/src/hook.rs`, `crates/cargo-tile/src/root_scan/`, `crates/cargo-tile/src/census/scan.rs` — their test modules, only when a measured test there still takes 1 s or more after the shared Mac start delay is removed (showrunner, 2026-11-06, from handler's Phase 65 measurement on main).

**Workers:** 1 writer — `test`, the listed test modules and their helpers.

**Constraints from prior phases:** Keep Phase 54's three named waits under the scenario deadline. Shorten the required work; do not restore fixed readiness or writer-exit limits.

**Acceptance gate:** the Mac and natedev suites show no test at 1 s or more; no assertion is weakened.

### Phase 43e — Mac tests start without a stall  · status: done

#### As-built

- Reader scenarios copy the test executable into a cache outside the build directory. A hard link shares the inode with the build output, and the Mac stalls test starts on it. Mac suite time fell from 22-43 s to 9-15 s at load 4-6. Partial delivery: the remaining Mac reader-test times under load and the Mac compiler CPU excess over its kernel counter belong to "Mac reader tests stay under 1 s under load".
- `setup_reader.py` (`crates/cargo-tile/src/shim_registration/`) owns the cache path. As the nextest wrapper and setup script it stages the reader copy and loads it once on the Mac. Copies are named for their live run owner and the content digest; publication writes the hashed bytes through a temporary file and renames it; a later staging removes copies of retired owners. `reader_executable.rs` stages through the same script outside nextest.
- `run_with_capture_parent` (`terminal.rs`) takes `before_first_scan: impl FnOnce()`, called after app setup and before the scanner starts. Reader tests hold their reader-start barrier there; product `run()` passes a no-op.
- `GridMotion::settle_if_immediate` runs before `tiles.tick()` in the terminal poll. It settles `Immediate` motion, which reader scenarios select; the product's `Animated` motion makes it a no-op.
- `reader_scenario.py` is the launch wrapper. The scenario cases and the CPU oracle are in `reader_scenario_cases.py`.
- Test cuts: queued CPU frames, fewer shell launches in fixtures, `python3 -S`, fewer serial waits in the hook tests, and an in-memory inventory cutoff test at `CAPTURE_INVENTORY_LIMIT`.

**Files:**
- `crates/cargo-tile/src/shim_registration/setup_reader.py` — reader cache path, staging, preload and retired-copy removal.
- `crates/cargo-tile/src/shim_registration/reader_executable.rs` — reader staging outside nextest.
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — reader tests, `Immediate` motion, the reader-start barrier in the hook.
- `crates/cargo-tile/src/shim_registration/reader_scenario.py`, `reader_scenario_cases.py`, `reader_staging_cases.py`, `fixture_events.py`, `capture_log_observer.py` — scenario launch, cases, oracle and staging tests.
- `crates/cargo-tile/src/terminal.rs` — the `before_first_scan` hook and the settle position.
- `crates/cargo-tile/src/hook.rs`, `progress/capture.rs`, `render.rs`, `root_scan/`, `shim_registration/installed.rs` — test-module cuts.
- `.config/nextest.toml` — the reader setup script and wrapper, and the `cargo-tile-readers` group.

**Binds later work:** only reader tests use the reader cache; its atomic publication, live-owner retention, retired-copy removal and the four staging regression tests stay. `before_first_scan` stays after app setup and before the scanner starts. `settle_if_immediate` stays before `tiles.tick()`. Reader PTY tests and settled helpers do not prove animated motion.

**Gotchas:** uninstrumented Mac test times follow host load (load 12-35 put 60+ tests at 1-3.8 s); compare only by alternating pairs. Every-slot reader scheduling made the Mac suite 4.1-8.2 s slower.

**Ruled out:** a hard link of the test executable (it causes the start stall); age-only eviction of reader copies (age does not prove disuse); a test-only inventory limit in `root_scan` product code (the in-memory cutoff test replaces it).

### Phase 43d — Capture text and constants follow the style guide  · status: done

#### As-built

- `CargoTransport::from_argv` binds the unused capture parent as `_` (pattern `[program, flag, executable, _, arguments @ ..]`).
- Both `From<CargoArgvAbsence>` implementations call `CargoArgvAbsence::map_reason(self, argv_unavailable, program_rejected)`, which maps `ArgvUnavailable` and `ProgramRejected` to the target's reasons.
- The capture shim constants in `constants.rs` are in alphabetical order.

**Files:**
- `crates/cargo-tile/src/census/command_text.rs` — `CargoTransport::from_argv`, `CargoArgvAbsence::map_reason`.
- `crates/cargo-tile/src/constants.rs` — the ordered capture shim constants.

### Phase 43h — Mac reader tests stay under 1 s under load  · status: done

#### As-built

- The stopped-compiler check reads the independent kernel CPU counter after two later scan records complete, while the compiler is still stopped. The strict bound `recorded <= independent + 0.000000001` stays.
- Nested-command readiness checks follow the reader barrier release, so they overlap the first scan; every readiness assertion stays.
- The incomplete-log sweep test fills its `Inventory` in memory through the test-only `scan_with_incomplete_logs` helper.
- The Darwin birth-conversion fixtures start `python3` with `-S`.
- The `InvocationLifetimeReads` doc comments put the type name in backticks for the Mac `doc_markdown` lint.
- No product state or user control changes.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py` — the stopped-counter order and the overlapping readiness checks.
- `crates/cargo-tile/src/root_scan/inspected_directory.rs` — the test-only `scan_with_incomplete_logs` helper.
- `crates/cargo-tile/src/root_scan/sweep_authority.rs` — the sweep test on the in-memory inventory.
- `crates/cargo-tile/src/shim_registration/installed.rs` — `python3 -S` in the conversion fixtures.
- `crates/cargo-tile/src/census/scan.rs` — the corrected Mac-only doc comments.

**Binds later work:** "The Mac census reads executable names through libproc" owns the two remaining slow reader tests (the refresh sample in `census/scan.rs`). "The install lifecycle test leaves no process behind on the Mac" owns the locked-uninstall time (CLI startup and product lock retries) and the leaky `caller_descriptors_reach_captured_cargo`.

**Gotchas:** On the Mac, `SSTOP` can show before the final CPU charge is visible, so read the counter only after the stopped scan completes. The Mac `doc_markdown` lint covers Mac-only code that natedev lint does not compile. Under load, up to three Mac tests can still reach 1 s; natedev has none.

**Ruled out:** copying product installation logic into the lifecycle fixture; a wider CPU assertion bound.

### Phase 43f — The Mac census reads executable names through libproc  · status: done

#### As-built

- `prepare_fixture()` uses `/bin/bash` symlinks for the Mac `excluded` and `quiet-json-long` scenarios.
- Other scenarios retain copied shells, with codesigning on the Mac.
- Existing nested-command readiness checks complete before `release_reader()` starts the first scan.
- The excluded and quiet-JSON Mac tests take 0.407–0.993 s and 0.301–0.542 s in the recorded suites.
- The Mac census retains sysinfo executable refreshes through `with_exe(UpdateKind::Always)`.
- The recorded gate passes 1199 Linux tests and 1215 Mac tests twice.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py` — scenario shell selection and readiness before reader release.

**Binds later work:** “The install lifecycle test leaves no process behind on the Mac” preserves these fixture constraints.

**Gotchas:** The cache-server fixture requires the copied shell for its named `sccache` and `rustc` hard links.

**Ruled out:** `proc_name` as executable spelling; a change-mark cache without a CPU gain beyond measurement spread.

### Phase 43g — The install lifecycle test leaves no process behind on the Mac  · status: done

#### As-built

- No test in `cli_lifecycle`, `control_s_saves_a_favorite` or `caller_descriptors_reach_captured_cargo` is leaky in two Mac and one natedev no-cache full suites.
- The reader scenario fixtures (`fixture_events.py`, `reader_scenario_cases.py`) keep one file subscription per path through each wait. One traversal registers and refreshes the watches. A native delete event retires a stale watch, so a path created again gets a new watch.
- `InstalledShim::repeat_in_same_shell` (`installed.rs`) makes the per-invocation observation folders in Rust, links with native `ln`, and skips the state listing on the repeat path.
- `interrupt_to_the_process_group_lets_cargo_finish_first` (`tests/shim_modes.rs`) sends a second interrupt while cargo stops, so the shutdown race shows. The cargo-finished assertion stays.
- The timing acceptance is not met. The showrunner ruled (2026-11-08) that the phase lands at the three-round repair cap because it beats main in the same hour: tests at 1 s or more were 1, 6 and 6 against main's 4, 18 and 12. `reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` (1.082 s natedev) and `reader_excludes_a_live_command_without_sweeping_its_capture` (1.049 s and 1.190 s Mac) go to the phase "Intermittent test faults get their owners, and two more slow tests go under 1 s", with the leak owners, the interrupt race cause and the CLI startup cost.

**Files:**
- `crates/cargo-tile/src/shim_registration/fixture_events.py` — retained watches, one traversal, renewal of retired watches.
- `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py` — scenario waits on the retained watches.
- `crates/cargo-tile/src/shim_registration/installed.rs` — the `repeat_in_same_shell` fixture.
- `crates/cargo-tile/tests/shim_modes.rs` — the second interrupt during cargo shutdown.

**Binds later work:** the reader fixtures keep each subscription through one wait, renew retired watches, and close subscriptions on every exit. Keep `repeat_in_same_shell` with both live log observations, and the second interrupt with the cargo-finished assertion.

**Gotchas:** nested commands get `CARGOTILE_NESTED` from the product shim, never from the fixture. The CPU readings must span 250 ms, with three sustained readings and the stopped-compiler checks. Stage times from a reverted attempt (Mac scan gap 865 ms, CPU baselines 671 ms, frame wait 1173 ms) locate waits but do not separate scan, render and scheduling cost; the kernel-worker and frame-observer durations overlap and do not add.

**Ruled out:** overlapping the serial scenario waits by setting `CARGOTILE_NESTED` in the fixture and timing from an earlier native origin; it lets the reading span fall to 100 ms and hides a broken product export.

### Phase 43i — Intermittent test faults get their owners, and two more slow tests go under 1 s  · status: done

#### As-built

- `NextRefresh::deadline(pace)` in `census/scan.rs` returns the active or idle refresh instant, and `NextRefresh::start` uses it. While it waits for a refresh, `ScanWorker::scan` sleeps until `min(next refresh deadline, now + poll)`. The scan pace, the readings and the shown values do not change; only idle wakeups drop.
- The Darwin membership fixture's `observe-account` in `shared_capture.rs` is a bash script with no Python process. It records uid, gid, arguments and environment, and forwards both output streams and the exit status.
- The `settings-scroll-burst` reader scenario uses the native `/bin/bash` symlink on Darwin, as `excluded` and `quiet-json-long` do.
- natedev full suites have no cargo-tile test at 1 s or more. On the Mac, `reader_keeps_parent_family_across_its_only_childs_source_switch` stays at 1.2-1.7 s under load; that test is not met here.

**Files:**
- `crates/cargo-tile/src/census/scan.rs` — the refresh deadline and the scan worker's wait.
- `crates/cargo-tile/src/shim_registration/shared_capture.rs` — the membership fixture observer.
- `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py` — the Darwin native shell for the settings scroll scenario.

**Binds later work:** a `census/scan.rs` change keeps the deadline sleep in `ScanWorker::scan` through `NextRefresh::deadline`.

**Gotchas:**
- The Mac source-switch fixture's readiness cost (about 400 ms) is outside the files above: native capture exec to constructor, and native fixture preparation on first use.
- A probe that writes a log before its timestamp, or injects a library, adds its own cost; its startup number is not a product number.

### Phase 43j — The Mac source-switch reader test runs under 1 s  · status: todo

**Depends on:** Phase 43i

Phase 43i landed at its 3-round cap by the count rule (director, 2026-11-09). Its round 3 head had 0, 1 and 1 cargo-tile tests at 1 s or more (natedev, Mac, Mac). The same-hour base had 0 and 5 (natedev, Mac; `phases/43i/base_compare.txt`, Mac suite 1; Mac suite 2 was cancelled). Its finding F001 stays open and moves here, owner tile, due at this phase's checkpoint.

#### Work Order

**Goal:** `reader_keeps_parent_family_across_its_only_childs_source_switch` and `reader_scenario_sigkill_ends_its_waiting_writer_group` run under 1 s in a loaded Mac full suite.

**Spec:**
1. The test took 1.707 s and 1.160 s in the two 43i round 3 Mac suites (load 13-19), and 0.413 s on natedev (`phases/43i/fix3_summary.md`). Its stage split (`phases/43i/fix3_stages.md`) names two costs outside the 43i files:
   - Native capture exec call to constructor: 238 ms.
   - Native fixture preparation: 168 ms, on the path `hook::fixture_runner` → `native_fixture::executable` → `setup_reader.py native-fixture`. `native_fixture.rs:31` reads the receipt; its cold branch starts `setup_reader.py` at line 38, which strips and signs the fixture on a cache miss (`setup_reader.py:87`).
2. Measure first. The 43i probe does not isolate either cost: it logs before its constructor timestamp and injects a library (Astra review, `/tmp/claude/astra/20261109T072032Z-2693821/review.md`).
   - Compare launches with and without the probe, at the same executable and load. Take monotonic timestamps before any log write. Time first and repeated launches apart.
   - Split preparation into receipt hit, artifact cache hit, lock wait, strip and sign. Later preparations in the same file took 0.380 ms and 0.166 ms, so 168 ms may be a first-use cost only.
   - Record the full source-switch timeline in loaded suites, with an empty and a populated fixture cache.
   Then fix the largest measured cost at its cause.
3. Keep every assertion and readiness wait. Keep the product `CARGOTILE_NESTED`, the 250 ms span of CPU readings, the three sustained readings and the stopped-compiler checks. Do not change the product scan pace.
4. `progress::capture::tests::unknown_staging_inspections_leave_allowance_for_ended_pairs_across_roots` took 1.009 s on the Mac in a 43i round 2 suite. Its largest stage starts at `progress/capture.rs:2088` (`phases/43i/fix3_stages.md`). Measure it again; if it is at 1 s or more in a gate suite, fix its cause.
5. If preparation moves to suite setup, give setup the product executable path (`CARGO_BIN_EXE_cargo-tile`), not the `unit_tests` executable it receives now (`setup_reader.py:118`). Publish the prepared path to Rust atomically, keep the source identity checks, and keep the direct-run fallback. Preload the final signed artifact with a harmless call such as `--version`. Report setup time apart from test time.
6. `copy_named_shell` (`reader_scenario_cases.py:313`) copies and signs the named shell for each source-switch fixture. If the timeline shows this cost, prepare the shell once. Keep its executable name, the carrier's process identity, guardian ownership and readiness waits. Preload the artifact the test runs; a later copy makes a new inode.
7. The Phase 32 round 1 Mac suite (2026-11-09, `phases/32/repair_summary_round1.txt`) had twelve `shim_registration::installed` tests at 1.458-1.570 s, all finishing together, and one leaky test, `cli_lifecycle::tests::uninstall_waits_on_a_locked_temporary_orphan_before_classifying_it`. Phase 32 does not touch these files. Measure them in a loaded Mac full suite. Fix the shared cost at its cause, and stop the process that outlives the leaky test. Do not raise a limit.
8. `shim_registration::reader_scenarios::reader_scenario_sigkill_ends_its_waiting_writer_group` took 3.349 s in the Phase 59 Mac gate (`phases/59/mac_gate_r3.log`). The four ownership tests took 0.57-0.69 s in the same suite. Measure startup and cleanup apart, then fix the largest cost at its cause. Keep every assertion at `reader_scenario_cases.py:2024-2065`.

**Files:**
- `crates/cargo-tile/src/shim_registration/installed.rs` — item 7, test module only.
- `crates/cargo-tile/tests/cli_lifecycle.rs` — item 7, the leaky test.
- `crates/cargo-tile/src/hook.rs` — only `fixture_runner` and the native capture entry's startup.
- `crates/cargo-tile/src/shim_registration/native_fixture.rs` and `crates/cargo-tile/src/shim_registration/setup_reader.py` — the fixture preparation path.
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — the test.
- `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py` — items 6 and 8, the named-shell preparation and the SIGKILL scenario.
- `crates/cargo-tile/src/progress/capture.rs` — item 4, test module only.

**Workers:** 1 writer — `test`.

**Constraints from prior phases:** the Phase 43i constraints hold. Phase 40 replaced the sh/tee stderr mirror with a native pump thread in `hook.rs`; keep it. Phase 59: a reader scenario runs under an owner process that holds a lifetime pipe (`reader_scenarios.rs:180-222`). A group registers before its session is created, and the owner ends the groups before it removes the folder. Keep this order and the four ownership tests (`reader_scenarios.rs:265-286`).

**Acceptance gate:** two Mac full suites and one natedev full suite, `--no-cache`, with each run of the item 1, item 4, item 7 and item 8 tests and of `reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` (1.147 s in 43i round 3) below 1 s, and no leaky test; lint.

### Phase 43 — The reader uses less CPU under load  · status: folded into Phase 43a

**Depends on:** Phase 32m

Showrunner request (2026-10-29): the installed reader (main 043dc8c6, phase 32m's CPU fix) uses 63.2% of one core on natedev at load 124 in a 393×89 pty (15 s warmup, then 60 s). The reader before 32m used 59.7%. A restart does not help. This phase profiles the reader under load, then cuts the largest costs. It goes ahead of phases 30m, 35m and 39.

#### Work Order

**Goal:** the reader's CPU under load is at most half of today's number, with the same screen.

**Spec:**
1. **Profile first.** Run the reader built from the base commit under load on natedev with the session's pty probe (phases/43/cpu_probe.py: 393×89, 15 s warmup, then 60 s) and a sampling profiler (`perf record -g` or `samply`). Write the top costs, each with its share of the reader's CPU and its call path, to phases/43/profile_base.md before any code change.
2. **Cut the largest costs.** Remove work that repeats with no change in its inputs: for example, a full redraw, layout or census pass when nothing changed since the last frame, or a refresh that reads data the screen does not show. Each cut keeps its rule in the types (a frame or scan that knows it has no new input cannot do the work), never a procedural flag check scattered over call sites.
3. **Same screen.** The drawn frames do not change: main's layout, colors, fades and deadlines stay as they are.
4. Profile again after the cuts with the same probe and profiler. Write phases/43/profile_cut.md with the same table.

**Files:** decided by the profile. The likely set is in the reader's frame and census paths: `crates/cargo-tile/src/render.rs`, `roster.rs`, `terminal.rs`, `main.rs`, `census/scan.rs`, `census/cpu_observation.rs`. Name the final set in the done post.

**Workers:** 1 writer (`impl`).

**Constraints from prior phases:** work in the isolated copy /tmp/claude/scratch/phase-tile-unit-43-5qlvls/src (git; base commit 2e0cc5a = branch commit 1e066ba6). Every assertion on main stays. No test of 1 s or more. No new allow or expect.

**Acceptance gate:**
- On natedev, the probe run back to back with the base build, under the same load: the cut build's CPU is at most half of the base build's.
- New logic has tests, red on the base first, each under 1 s.
- `cargo nextest run -p cargo-tile -p tui_pane`, workspace clippy with `-D warnings`, and `cargo +nightly fmt --check` green (natedev, `verify.sh` with `--local` while the Mac is blocked; the Mac after 00:05).
- A live capture against main shows the same layout.

### Phase 38 — A new cell's text starts near its final color  · status: done

User report (2026-10-29, via the showrunner): a new cell's text fades in from black and then goes away so fast that it cannot be read. The first frame of a new cell's text is drawn near its final color, not near the ground. Exact-color test red on main first. Work order: phases/38/implementation_prompt.md.

### Phase 39 — The mem value counts every compile's memory  · status: done

#### As-built

- Each cargo row's mem includes the compilers its sccache clients requested. A rustc that the sccache server runs goes to the cargo whose tree holds the matching client: `CompilerRequest::{Compilation, DependencyDiscovery, OutputDiscovery}` (`census/invocation_cpu_accounting.rs`) keys each request by its normalized arguments, and `matching_clients` finds the clients. The `--out-dir` under a target directory rule is the second rule, when no client matches.
- `CompileMemoryOwner::{Requesting(Pid), ByTargetDirectory(Pid), Unattributed(UnattributedCause)}` (`census/scan.rs`, `detached_compilers`) names each owner, with no fallthrough. `UnattributedCause` is `SharedTargetDirectory`, `NoTargetDirectory`, `NoOutDir`, `NoUid`, `NoRequest` or `SharedRequest`.
- The sccache server's own RSS is one "shared compiler cache" share, and memory with no owner is one "unattributed" share. Each is counted once per scan and shows in the summary only when not zero (`SUMMARY_SHARED_COMPILER_CACHE_LABEL`, `SUMMARY_UNATTRIBUTED_MEMORY_LABEL`).
- The cargo mems plus both shares equal the RSS sum of the assigned processes. `SummaryFootPreparation` carries both shares, so a change to a share alone repaints the `mem` foot.

**Files:**
- `crates/cargo-tile/src/census/scan.rs` — owner rules, `CompileMemoryOwner`, the shares.
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs` — `CompilerRequest`, `matching_clients`.
- `crates/cargo-tile/src/census/compile_memory_tests.rs` — one test per owner case and the totals test.
- `crates/cargo-tile/src/constants.rs` — the rustc and sccache argument words and the share labels.
- `crates/cargo-tile/src/terminal.rs`, `roster.rs`, `render/frame_preparation.rs`, `render.rs` — the shares from each scan to the summary foot.

**Gotchas:** sccache runs a dependency discovery compile (`--emit=dep-info` to one temporary file) and an output-name probe (`--print=file-names`) besides the compile itself. Each matches its client only through its `CompilerRequest` kind.

### Phase 40 — Capture falls back to plain cargo when its stderr mirror cannot run  · status: done

#### As-built

- The native capture entry copies cargo's stderr through `NativeStderrPump` in `hook.rs`, a thread named `cargo-stderr`. It writes each read to the live log and to the caller's stderr. A failed log file or caller stderr does not close cargo's pipe; the other output still receives the bytes.
- `NativeStderrPump::start` is a fallible constructor that runs before cargo spawns. If it fails, the capture closes what it opened, removes its publication and execs the original cargo with its own arguments, streams and status. Cargo never runs twice.
- No `sh` or `tee` process runs for capture. A captured call starts two processes.

**Files:**
- `crates/cargo-tile/src/hook.rs` — `NativeStderrPump` and the fallback to plain cargo.
- `crates/cargo-tile/src/constants.rs` — `CAPTURE_STDERR_PUMP_BUFFER_BYTES` (8192).
- `crates/cargo-tile/tests/shim_modes.rs` — rows with `sh` present and `tee` absent (native capture runs) and with a descriptor limit (the pump fails and cargo runs plain, with no capture artifacts).
- `crates/cargo-tile/src/shim_registration/installed.rs` — the live log handshake test expects status 37 with the live marker while cargo runs.

**Gotchas:** an exec acknowledgement followed by EOF is uncertain; the fallback never retries cargo.

### Phase 42 — A delayed cell decides its entry on the frame its text first shows  · status: done (merged c1efe2543)

**Depends on:** Phase 38

From phase 38's live check (routed at repair round 4; phase 38 landed because it beats main on every live count: delayed cells readable 3 of 6 on the branch against 0 of 8 on main, fades and deadlines as main). A cell that opens from the foot of its column first paints only its column-header row. Phase 38 decides the cell's entry on that frame, so when the alpha there is 60–63 the cell takes main's fade, and its command text appears 1–2 frames later below 75% and fades out (live b1, cells s3–s5, first text frame 0.731).

**Spec (Astra review, phase 38 F003):** the entry decision is made only by consuming a private `VisibleCommandText` value, which only the layout and pane visibility mapping can construct when a nonempty command fragment survives the pane's clipping (`tui_pane` frame.rs scratch-buffer copy). Each tracked row has its own state `AwaitingText`, `MainFade` or `Brighten { started }`; a child's entry is not consumed by its lead. Ink reads (`for_painting`, the readout) are pure: while entry is pending they preview the candidate ink and commit nothing. Classification uses the row's original fade, not the painted copy. Expiry stays where it is.

**Brighten (user, 2026-10-30; change review row C152 returned; Phase 45 merged here):** "i didn't want full color i wanted 75% of the full color and it goes to 100% over a few frames". There is no hold at full ink. A late-entry cell's text starts at 75% of its color on the first visible text frame and brightens to 100% in about 250 ms (one value, the director's call), then follows its ordinary lifetime. Cells readable at first paint keep main's fade. One owner for the entry state; no placeholder value and no copied state. Repaints are asked for only while a brighten runs; when the last brighten ends, the reader returns to main's idle frame rate with no extra frames.

**Acceptance:** a staged-reveal test (headers at alpha 62, command text first shown at alpha 68) gives exact RGB at 75% on the first text frame, mid-brighten, at 100% at its end and just before the unchanged deadline, and no repaint request after the brighten ends; a clipped-in-scratch case, a hidden child and text first shown below the threshold keep their own rules; red first, each under 1 s; live capture vs main: each late-entry cell starts at 0.75 (±0.02) and reaches 1.00, the readout ink equals the text ink, frame count and reader CPU over the same workload, branch vs main (no frames after the brighten beyond main's).

**Files:** `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/tui_pane/src/pane/frame.rs`. Not `crates/tui_pane/src/tiles/draw.rs` or `tiles/grid.rs`: the handler unit's equal-frame phase owns them (showrunner, 2026-10-30).

### Phase 47 — A command cell's account and directory are its border title  · status: done

**As built** (checkpoint `ac47cbba`, merged as main `2fb29335c`, 2026-11-03): a command cell names its account tag and directory in its top border through `TileGroupTitle::Titled`. The body draws no lead heading row, so the cell shows one more process row. One account decision, `TableAccountHeadings`, feeds the title and any nested body heading. The directory cut fits the title room. A nested directory keeps its body heading. Live at 393x89 against main: 8 of 8 command cells have one title, no lead heading stays in a body, and process rows stay 17 of 17 with no clipped cell.

### Phase 47a — The border title is only the account and directory; progress is the first body row  · status: done

**As built** (checkpoint `229b35851`, merged as main `48dfe94d2`, 2026-11-03): a command cell's top border holds only its account tag and directory. Its progress reading (state word, meter, percent) is the first body row, above the table header. A cell with no reading starts with its table. One owner computes the reading row, and a cell's demand counts it only when it draws. Live at 393x89 against main: 0 of 8 titles hold progress text (main 8 of 8), 8 of 8 readings are the first body row, process rows stay 17 of 17, and no cell is clipped. Gates under the bump: lint and 15 related tests; the showrunner runs the full suite on the merged tip.

### Phase 47b — The cell border is plain; the account and directory are the first body row  · status: done

**Depends on:** none (it starts on main `4b3ce44da`, after Phase 47a)

#### Work Order

**Goal:** A command cell shows its border as a plain line again. The account and directory move into the body. From the user (2026-11-03, through the showrunner): "it works - i think i miss seeing the cell borders - rather than have the title be the [username] - {direcotry} - i think it should just be the first row - followed by the optional build/test progress bar. hopefully that's simple to change". The showrunner put this phase first in the order.

**Spec:**
1. The top border of a command cell carries no title. It is the plain border line that cells had before Phase 47.
2. Body row 1 shows the account and directory, for example `[natepiano] ~/rust/hana`. It uses the one account decision, `TableAccountHeadings`, and the directory cut that fits the row.
3. Body row 2 shows the progress reading (state word, meter, percent), only when the command has one.
4. The process table follows. A cell with no reading starts its table on row 2.
5. A cell's demand counts the heading row always and the reading row only when it draws.

**Tests, red first:**
- `a_command_cell_border_has_no_title`: the top border row of a command cell holds only border glyphs.
- `a_command_cell_names_its_account_and_directory_on_its_first_body_row`: body row 1 reads `[account] directory`.
- `a_command_cell_draws_its_progress_on_its_second_body_row`: with a reading, body row 2 holds the state word and meter; without one, row 2 is the table header.
- Change the Phase 47 and 47a assertions that expect the title. Each Assertion-Change line names this user request as its reason.

**Files:**
- `crates/cargo-tile/src/render.rs`
- `crates/cargo-tile/src/constants.rs`
- `crates/cargo-tile/CHANGELOG.md`

**Workers:** 1 writer (`impl`). The CHANGELOG gets one Changed line.

**Acceptance gate:**
- `verify.sh lint cargo-tile` and the related `cargo-tile` tests green; no test takes 1 s or more.
- Unit director's live capture at 393x89 against main: every command cell has a plain top border, body row 1 is the account and directory, the reading is row 2 when one runs, and no cell is clipped.
- Change-review line: a command cell's border is plain, and its account and directory are its first body row.

### Phase 52 — The attract screen is a cargo-tile feature, on by default  · status: done

**Depends on:** none (it starts on main)

#### Work Order

**Goal:** A build of cargo-tile with no default features links no Swift code. The showrunner (2026-11-05) put this phase second in the order. Nix on the Mac builds cargo-tile with `buildNoDefaultFeatures` for the hana CI hook. Today `crates/cargo-tile/Cargo.toml` turns on the `tui_pane` `backdrop` feature with no condition. `backdrop` pulls `screencapturekit`, and its `apple-cf` dependency needs Swift. That failed the natedev darwin rebuild `8d0a2ca`. Natedev and resources wait on this checkpoint.

**Spec:**
1. cargo-tile has a new feature, `attract`, in its default features. `attract` turns on `tui_pane/backdrop`.
2. The `tui_pane` dependency of cargo-tile no longer turns on `backdrop` itself. Only `attract` does.
3. The attract code compiles only with `attract`: its keys, constants, interaction, global action, probe, app state, terminal setup and drawing.
4. With `attract` off, cargo-tile builds and runs. It has no attract screen and no attract key, and the help shows no attract key.
5. With default features, cargo-tile behaves as it does on main.

**Tests:** no new test. The default-feature suite proves item 5. The two builds in the gate prove items 1 to 4.

**Files:**
- `crates/cargo-tile/Cargo.toml`
- `crates/cargo-tile/src/keymap.rs`
- `crates/cargo-tile/src/constants.rs`
- `crates/cargo-tile/src/interaction.rs`
- `crates/cargo-tile/src/app_global_action.rs`
- `crates/cargo-tile/src/probe.rs`
- `crates/cargo-tile/src/app.rs`
- `crates/cargo-tile/src/terminal.rs`
- `crates/cargo-tile/src/render.rs`
- `crates/cargo-tile/CHANGELOG.md`

**Workers:** 1 writer (`impl`). The CHANGELOG gets one Added line for the `attract` feature.

**Acceptance gate:**
- `cargo build -p cargo-tile --no-default-features` passes on the Mac, and the link has no Swift library.
- The default build passes on natedev and on the Mac.
- `verify.sh lint cargo-tile` and the `cargo-tile` tests green; no test takes 1 s or more.
- Change-review line: cargo-tile builds with no default features on the Mac, without Swift.

### Phase 46 — Enter shows the focused cell on the full screen, and Esc returns to the grid  · status: done

#### As-built

- Enter grows the focused cell (the summary or a command cell) from its grid rectangle to the full grid area with the grid's cell travel clock and easing; Esc shrinks it to the rectangle the grid gives it on that frame, and focus stays on the same cell. A cell whose command ends returns the view to the grid.
- `GridView::{Grid, Opening, Expanding, Expanded, Collapsing}` (`crates/cargo-tile/src/grid_view.rs`) owns the view state. `Opening` accepts Enter before the first expanded paint; keys the expanded cell does not consume stay Unhandled there, so global keys such as `q` still work (private `CellKey` classification).
- `ExpandedCell` (render.rs) implements `TileCells` and uses the grid's one cell draw path in the larger rectangle. Up/Down/PageUp/PageDown/Home/End scroll its rows. `CellColumnHeader::{Absent, Row(u16)}` (`render/cell_paint.rs`, carried in `CellPaintReach` and `PaintedCellContents`) records the painted header row; `ExpandedCell::draw` copies it onto the first inner row, the footer stays on the last, and the frame readout shows the shown range without the header.
- Expanded preparation keeps its own retained frame with shared command measurements (`render/frame_preparation.rs`); frame preparation inputs include the `GridView`, the expanded identity, the scroll and the motion time.
- Toast placement protects the painted summary frames and the moving cell during `Expanding` and `Collapsing`, and the expanded cell alone in `Expanded`.
- Status bar: `enter expand` (grid) and `esc collapse` (expanded) with two cells on both sides; `GlobalSlotFit::AfterUptime` (`tui_pane` bar/status_line.rs) marks a hint slot that gives way before uptime; `? shortcuts` keeps its place with one trailing cell. One hint styler in `tui_pane` bar/support.rs serves keymap hints and the fixed `esc collapse` hint. The keymap has `global.expand = enter` ("Expand the focused cell").

**Files:**
- `crates/cargo-tile/src/grid_view.rs` — `GridView` and its clock.
- `crates/cargo-tile/src/render.rs` — `ExpandedCell`, toast protection, status hints, view tests.
- `crates/cargo-tile/src/render/frame_preparation.rs` — the expanded preparation path.
- `crates/cargo-tile/src/render/cell_paint.rs` — `CellColumnHeader`.
- `crates/cargo-tile/src/app.rs`, `terminal.rs`, `keymap.rs`, `app_global_action.rs` — key routing and the expand action.
- `crates/tui_pane/src/tiles/grid.rs`, `tiles/draw.rs` — the additive travel method for one cell.
- `crates/tui_pane/src/bar/status_line.rs`, `bar/support.rs` — `GlobalSlotFit` and the one hint styler.

**Binds later work:** pause holds the `GridView` clock as well as the grid clock; the one target layout includes expand and collapse, whose collapse target is the grid rectangle on that frame; an expanded ended cell returns to `Grid`; table changes keep `ExpandedCell`'s full content and pinned header; a disk or summary change reaches both preparation paths; a new status-bar hint uses `GlobalSlotFit` and the one hint styler.

**Gotchas:** the event loop drains input bursts before it draws, so a transient view state must not consume global keys. Expand motion repaints 19-35 frames against 8 for ordinary travel: 0.39 s against 0.29 s CPU in colour, 0.46 s against 0.34 s in NO_COLOR.

**Ruled out:** a screen label for `Opening`.

### Phase 30 — The capture shim costs one process per cargo call  · status: folded into Phase 30m

**Depends on:** none (Phase 28 was dropped, 2026-10-29). It does not depend on Phase 29: its files are not the files of Phase 29's splits, thus it can run beside Phase 29.

#### Work Order

**Goal:** The cargo-tile capture shim starts at most one process for each cargo call, apart from cargo and `tee` (showrunner, 2026-10-28). The shim (version 3, which `cargo-tile install` writes) costs about 0.087 s for each cargo call on the Mac, outside cargo. That cost comes from about 43 helper processes for each call (line numbers on main): `ps -Ao` (line 124), `find` (235), `chmod` (242, 249, 325, 342), `sysctl kern.bootsessionuuid` and `ps -o lstart` (294 to 297), and `tee` (499). berth-unit measured 4,112 process starts in 20 cargo-mend tests.

**Spec:**

1. **Setup once for each boot.** The shim makes its directories and sets their modes once for each boot, not for each call.
2. **The boot id and the start time are read once.** The shim reads the boot id and the start time once and keeps them in a cache.
3. **No `ps` ancestor walk on each call.** Use `$PPID` in place of the `ps` ancestor walk, or skip the walk when no grid listens.
4. **Measure the cost.** Measure the cost for each cargo call on the Mac before and after the change, and record both in the Outcome. Each capture that the grid uses stays.
5. **Shim version 4.** The shim's version becomes 4, and `cargo-tile install` updates an installed version 3 shim.

**Files:**
- `crates/cargo-tile/src/cargo-capture-shim.sh` — items 1 to 3 and 5.
- `crates/cargo-tile/src/hook.rs`, `crates/cargo-tile/src/constants.rs` — item 5's install and version check, and their tests.
- `crates/cargo-tile/CHANGELOG.md` — one line for the faster shim.

**Workers:** 1 writer — `impl`, every file above.

**Constraints from prior phases:** Phase 28's Constraints hold, on its landed checkpoint. Each capture that the grid reads today stays. No new `#[allow]` or `#[expect]`; no test takes a second.

**Acceptance gate:**
- For each cargo call, the shim starts at most one process apart from cargo and `tee`.
- The cost for each cargo call on the Mac, before and after, is in the Outcome.
- `verify.sh lint cargo-tile`, then `verify.sh test cargo-tile`, green.
- At merge, the showrunner installs cargo-tile on both machines.
- Change-review line: none (no visible change), or the faster cargo call if a test can show it.

### Phase 56 — A checkpoint release frees only its own phase's reservation  · status: done

#### As-built

- Checkpoint release uses `reconcile_for_release` with the selected reservation and current worktree.
- `MergedRunEndings::CheckpointRequested` permits the selected shared reservation to checkpoint; sibling reservations keep their active lifecycle and claims.
- Merge observations retain `ReservationShares` coverage when the remaining group contains one reservation.
- Reconciliation refreshes active holders from committed lifecycles before it removes cover markers.
- Tests cover the real release command and repeated reconciliation with two and four shared reservations.

**Files:**
- `crates/cargo-berth/src/verb/release.rs` — selects checkpoint reconciliation.
- `crates/cargo-berth/src/reconcile.rs` — controls checkpoint selection, active holders and sibling regression tests.
- `crates/cargo-berth/src/reservation/merge_extent.rs` — reads retained holder coverage.
- `crates/cargo-berth-test-support/fixtures/active_phase_claim.rs` and `shared_phase_checkpoint.json` — supply active sibling claims and shared ledger evidence.
- `crates/cargo-berth/tests/parallel_phases.rs` — checks release results and retained sibling claims.

**Binds later work:** “Three cargo-berth lock tests pass under load” and “cargo-berth tests run under a second” preserve the shared-reservation lifecycle tests.

### Phase 57 — Three cargo-berth lock tests pass under load  · status: todo

**Depends on:** Phase 56 (same writer and the same cargo-berth files). The Phase 30m merge does not wait on this phase.

The showrunner assigned this phase on 2026-11-08: a berth fault goes to the unit it blocks. The Phase 30m round 2 Mac workspace gate (run `20261108T100328-2152148-9d81`) failed two cargo-berth tests that Phase 30m does not touch.

#### Work Order

**Goal:** all six tests named in Spec items 1–4 pass on natedev and the Mac in a full workspace run, each under 1 s, by a fix of the cause in the code.

**Spec:**
1. `pre_tool_use_reports_reconcile_contention_as_busy_without_naming_a_holder` (`tests/hooks.rs:743`) failed its elapsed-time assertion at `tests/hooks.rs:798` (0.991 s). The hook answered "another cargo-berth operation still holds the ledger lock". The Phase 58 paired Mac head run (2026-11-09) failed it again with a ledger-lock refusal. Find the wait that takes the time, quote its line, and fix it there. The timer at `tests/hooks.rs:771` includes `FixtureCommand::output`, which uses the shared child runtime's Mac waiter thread. Compare the previous and current shared child runtimes under the same Mac load. Measure check execution, hook execution and output collection apart before you change a lock wait.
2. `explicit_retention_lock_diagnostic_identifies_a_process_holding_it_open` (`tests/ledger.rs:2527`) panicked at `tests/ledger.rs:2547` (2.412 s): the ledger read failed on a git `packed-refs.lock` of age 0.218 s, so the diagnostic named no process holder. Find what holds that lock in the fixture or in the product, quote its line, and fix it there.
3. `a_session_checks_its_held_path_without_waiting_for_the_mutation_lock` (`tests/lifecycle.rs:1493`) failed on natedev in the Phase 43i workspace gate (run `20261108T120132-2136765-d16a`, 1.264 s): "the shortened lock deadline must keep the proof under one second: 505.003612ms". Find the wait that takes the time, quote its line, and fix it there.
4. Three `tests/board.rs` tests failed on the Mac in the Phase 30m round 3 workspace gate (2026-11-08 22:1x, load 16-40): `a_deferred_comparison_withholds_the_alert_for_a_rewritten_ancestor_proof` (`tests/board.rs:6989`, left `Null`, right `"protected_tip_ancestor"`), and `a_standing_proof_is_reanchored_onto_a_trunk_that_moved_forward` and `a_deferred_comparison_withholds_the_alert_for_an_unevaluated_content_proof` (`tests/board.rs:7070`, "ordinary reconciliation should settle the reachable checkpoint", left `"outstanding"`, right `"released"`). Find the cause, quote its line, and fix it there.
5. Reproduce each fault red first on the machine that showed it, with the method that shows it (for example, a filtered run under the same parallel load). Quote the failing line.
6. Do not loosen an assertion. Keep every existing assertion.

**Files:**
- `crates/cargo-berth/tests/hooks.rs`, `crates/cargo-berth/tests/ledger.rs`, `crates/cargo-berth/tests/lifecycle.rs`, `crates/cargo-berth/tests/board.rs` — the tests and their fixtures.
- `crates/cargo-berth-test-support/` — shared fixtures.
- `crates/cargo-berth/tests/recorded_templates.rs` — registrations of the recorded fixture states for these tests (routed 2026-11-09).
- `crates/cargo-berth/src/git/error.rs`, `crates/cargo-berth/src/git/protocol.rs` — the `lsof` holder lookup in `git_lock_holders` (`error.rs:408`) runs under `CONTROL_REPLY_DEADLINE` (750 ms, `protocol.rs:21`); under Mac load it ends without a holder (routed 2026-11-09).

Another product file of a cause needs director routing.

**Constraints from prior phases:** Phase 56: shared merge coverage does not checkpoint a sibling. Keep reservation-and-worktree selection in `MergedRunEndings::CheckpointRequested`, active sibling claims and the repeated-reconciliation assertions. Hold `S/locks/mac.lock` until the detached Mac gate ends; `verify.lock` alone does not serialize gates. Phase 58a: merge its four registrations with `PREPARED_LOCK_REPOSITORIES` in `tests/recorded_templates.rs`. Keep both fixture module sets and `cold_single_target_setup_builds_every_recorded_template`.

**Workers:** 1 writer.

**Acceptance gate:** the cargo-berth package suite on natedev and on the Mac, `--no-cache`, and one Mac workspace run, with all six named tests green and each of them under 1 s; lint. The other slow cargo-berth tests in these four files go to Phase 58b.

### Phase 58 — cargo-berth tests run under a second  · status: done

#### As-built

- Integration tests restore recorded Git histories before their live operations.
- `LedgerTemplate::restore_into` renews owners and removes derived replay files.
- The shared child runtime uses exit notifications and collects active output pipes without threads for absent streams.
- Native reads use the requested checkout despite inherited selectors; unsupported inherited Git settings retain the subprocess route.
- The under-one-second goal remains unmet. Paired Mac counts fall from 406 to 330 overall, and from 217 to 168 within this phase’s scope.

**Files:**
- `crates/cargo-berth/src/git/child.rs` — exit notifications, pipe collection and cleanup.
- `crates/cargo-berth/src/git/read.rs`, `crates/cargo-berth/src/git/constants.rs`, `crates/cargo-berth/src/git/constants/child.rs` — native-read policy and shared constants.
- `crates/cargo-berth-test-support/src/` — the shared child runtime and history restoration API.
- `crates/cargo-berth/tests/` — recorded histories, fixture restoration and live test operations.
- `crates/cargo-berth/src/board/answers.rs`, `crates/cargo-berth/src/board/rows.rs` — shorter test setup and separate waiting-reason cases.

**Binds later work:** “Three cargo-berth lock tests pass under load” checks the shared runtime as a possible contention-cost source. “The remaining cargo-berth tests in Phase 58 files run under a second” preserves live operations and fixture renewal.

**Gotchas:** The paired Mac head fails the hooks contention test. The natedev count of 48 slow tests precedes restoration of the original compaction setup. Mac exit notification can require a waiter thread.

**Ruled out:** recurrence of the four earlier test-cost causes.

### Phase 58a — The remaining cargo-berth tests in Phase 58 files run under a second  · status: done

#### As-built

- Four recorded histories prepare compaction, rebase, answer and orphan prerequisites before the tested operations.
- The two rebase hook modes use separate tests and private history copies.
- Compaction readers, byte accounting, both compactions, answer transitions, orphan recovery, successor rotation and witness loss remain live.
- The under-one-second goal remains unmet.
- Gate counts at 1 s or more are natedev package 188 (load 46.73), natedev workspace 158, Mac package 416 and Mac workspace 574 (loads 9.60 and 18.47).
- A later natedev package run on the same tests has 18 tests at 1 s or more. That run did not record its load.

**Files:**
- `crates/cargo-berth/tests/{answers,compaction,edges,gate}.rs` — private history restoration and live test operations.
- `crates/cargo-berth/tests/support/{answers_fixtures,compaction_fixture,edges_fixtures,gate_fixtures}.rs` — prerequisite histories and shared command helpers.
- `crates/cargo-berth/tests/recorded_templates.rs` — registration of the four histories.

**Binds later work:** “Three cargo-berth lock tests pass under load” preserves both registration sets. “cargo-berth tests stay under a second when they run together” owns the remaining costs.

**Gotchas:** Slow-test counts require load and concurrency records for comparison. The gate measurements do not separate cold and warm fixture-cache costs.

**Ruled out:** a product pipe-collection repair based on these measurements.

### Phase 59 — A reader scenario stops its processes and removes its folder on every exit path  · status: done

#### As-built

- A reader scenario runs under an owner process that outlives it. The owner holds a lifetime pipe from the scenario.
- When the scenario ends, the owner ends every registered writer and cache-server group, then removes the scenario folder.
- This holds on a pass, a failed assertion, a deadline timeout and a SIGKILL of the scenario.
- Group selection works on Linux and macOS. An all-zombie group ends without an error. A stalled registration client cannot block the owner.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py` — the owner process and the four exit-path scenarios.
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — the lifetime pipe and the four ownership tests.

**Binds later work:** A group registers before its session is created. The owner ends the groups before it removes the folder. The four ownership tests in `reader_scenarios.rs` stay as the acceptance checks of this lifecycle. `copy_named_shell` in `reader_scenario_cases.py` copies `/bin/bash` and signs it with codesign on Darwin.

**Gotchas:** On the Mac, `reader_scenario_sigkill_ends_its_waiting_writer_group` takes 3.349 s. Startup of the scenario and its caller is most of that time. "The Mac source-switch reader test runs under 1 s" owns this time.

### Phase 58b — cargo-berth tests stay under a second when they run together  · status: todo

**Depends on:** Phase 57 merged, and Phase 58a checkpointed.

Phase 58a landed its fixture repairs with its goal open. Its gate passed on both machines. Tests at 1 s or more: natedev package 188 (load 47), Mac package 416 and Mac workspace 574 (loads 10 to 18). The Mac tests that took 1 to 3.5 s alone took 10 to 14 s under normal concurrency. One Git read costs the same direct and through the shared child runtime, and pipe collection after exit is negligible. The measurement report is `after-report.md` of the Phase 58a run.

#### Work Order

**Goal:** every cargo-berth test runs under 1 s in the full workspace run on natedev and on the Mac. A test of 1 s or more under load is a test defect. Fix the cost per test. Do not raise a limit, and do not call it a load flake.

**Open from Phase 57** (landed under the three-round rule, 2026-11-10; Mac package run `--no-cache`, load 14.51 on 12 CPUs, 6 threads; log `S/phases/57/mac_gate.log`):
1. `git::read::tests::installation_probe_seals_its_pipes_before_another_git_child_starts` failed on the Mac: "installation probe did not start". The child waits 250 ms of wall time for the shim's `ready` file (`src/git/read.rs:1172`). Remove the wall deadline from the proof.
2. Five Phase 57 tests are green but over 1 s on the Mac: lifecycle `a_session_checks_its_held_path_without_waiting_for_the_mutation_lock` 1.414 s; ledger `explicit_retention_lock_diagnostic_identifies_a_process_holding_it_open` 1.243 s; board `a_deferred_comparison_withholds_the_alert_for_a_rewritten_ancestor_proof` 1.526 s, `a_standing_proof_is_reanchored_onto_a_trunk_that_moved_forward` 1.277 s and `a_deferred_comparison_withholds_the_alert_for_an_unevaluated_content_proof` 1.199 s. On natedev all six are under 1 s.
3. Approach (Astra review, `/tmp/claude/astra/20261110T145250Z-307455/review.md`): measure fixture restore, process start, live commands, lock diagnostics and cleanup apart for each named test at 6 threads and Mac load 14 to 18. Replace the timing waits in the installation probe proof with an explicit event at the spawn boundary. Split the two ledger cases into two tests. Prepare the lifecycle foreign holder and the board competitor history in registered templates. The remaining board competitor is the one that the second deferred pass creates during the test (`tests/board.rs:4626`). Keep both deferred passes, both trunk advances and the live proof validations. The 406 to 330 counts are Phase 58 evidence at load 7 to 8.
4. Keep `PREPARED_LOCK_REPOSITORIES`, `ACTIVE_FIRST_SPLIT`, `UPPER_FIRST_SPLIT`, `LOWER_FIRST_SPLIT`, their registrations in `tests/recorded_templates.rs` and `cold_single_target_setup_builds_every_recorded_template`. The recorded first splits stay prerequisites. Marker processing and the second rebase (`tests/support/reanchoring.rs:1049`) stay live.
5. `PreparedProofHistory<Commits>` (`crates/cargo-berth-test-support/fixtures/prepared_lock_tests.rs:78`) names preparation, not a state. Replace it with an active-history type and a released-proof type. Settlement produces the released-proof type.
6. Each Mac gate record names the executable paths and build identities of the Mac test binaries and `cargo-berth`. A measurement from a binary without the candidate code does not count.
7. The Phase 57 repair makes `ExitNotification::wait_for_child` (`src/git/child.rs`) check the process after a notification deadline. natedev passed 10 of 10 runs with the repair and 10 of 10 without it, so no run proves that the repair removes the Mac failure. If a Mac run shows the six tests that Phase 57 names fail again, they come first in this phase with a Mac-side diagnosis (showrunner, 2026-11-10).

**Spec:**
1. **Find the cause first.** Read the attempts log for Phase 43c: Mac test processes stall 1.9 to 4 s in full runs and take 0.01 s alone. This can be the same cause. Measure what a concurrent test waits on, and quote the line that takes the time.
2. **Fix the cost per test:** fewer Git processes and shared fixtures. Keep the Phase 58a rules: no recorded result replaces a live rebase, proof rotation or witness-loss command.
3. **Cover the four Phase 57 files:** `board.rs`, `hooks.rs`, `ledger.rs` and `lifecycle.rs`.
4. Keep every assertion and every test case. Split a test only with a map from each new test to the original assertions and live transitions.
5. Measure baseline and final gate counts at the same test concurrency and declared load range on each machine. Record revision, load, test threads, cache state, setup duration and complete-suite duration beside each package and workspace count.

**Files:**
- `crates/cargo-berth/tests/` — the integration tests.
- `crates/cargo-berth/tests/support/` — test fixtures.
- `crates/cargo-berth-test-support/` — shared fixtures.
- `crates/cargo-berth/src/board/{answers,rows}.rs` and `crates/cargo-berth/src/git/read.rs` — test modules only, for measured slow unit tests.

A product change under `crates/cargo-berth/src/` needs director routing with the measurement that picks it.

**Constraints from prior phases:** Phase 58a: preserve its four recorded histories, private restores, owner renewal, worktree relocation and derived-cache removal. Keep both rebase modes, ordered compaction readers, byte accounting, both compactions, answer transitions, orphan recovery, successor rotation and witness loss live.

**Workers:** 1 writer.

**Acceptance gate:** on the final source revision, the cargo-berth package run and one workspace run on natedev and on the Mac, `--no-cache`, with every cargo-berth test under 1 s; lint.

### Phase 31 — Ended cells leave whole  · status: parked

**Depends on:** Phase 35m (both change `render.rs` and `render/command_cache.rs`).

**Parked (showrunner, 2026-11-09):** repair round 4 was the last round, and it ended red. No paired capture was run. Restart from the uncommitted copy tree `/tmp/claude/scratch/until-2026-11-16T0500Z-phase-tile-unit-31-dcj7s6/src` (base 9b85b0e). The copy's hold ends 2026-11-16 05:00 UTC. Its natedev gate gave 7430 pass and 6 fail:
1. `cargo-handler` `render::layout_stability_tests::added_and_removed_tiles_change_the_grid_allocation`
2. `cargo-handler` `render::narrow_width_tests::unchanged_account_inputs_keep_the_first_selected_destination`
3. `cargo-handler` `render::narrow_width_tests::account_refresh_keeps_the_destination_through_animation_frames`
4. `cargo-tile` `render::tests::entry_ink_tests::a_brightening_readout_matches_fractional_text_channels`
5. `tui_pane` `tiles::draw::tests::a_large_cell_frame_measures_each_content_allocation_once` (the Summary allocation is measured twice)
6. `tui_pane` `tiles::grid::tests::falling_tree_demand_keeps_the_first_selected_destination` (the second draw moves Summary from 42 to 26 rows)

Lint did not run: `cargo mend --fix` wrote code that does not compile and rolled back. The showrunner routed that mend defect to port. The phase holds no reservation.

#### Work Order

**Goal:** An ended cell shrinks on its text fade clock and always draws its heading row. It never paints as a headless row or as a bordered box with no text (Phase 24 second capture, `r24b_d40_overview_9.00`; first capture, `r24_d80_overview_9.00`; Phase 31 first attempt, `paired/p31_d80_ended_p22k00_f162_8.575`).

**Spec:**

0. **State model first (showrunner, 2026-11-08).** Before any code, the writer puts the closing cell's state model in this section: each state, the one record that owns it, and every path that constructs or changes it. Two records never hold the same fact. The showrunner reviews it before code starts.

1. **An ended cell shrinks with its heading (user, 2026-10-16: "tiles should start disappearing over the time frame that the text fades away - which can clip some of the text obviously if the cell is tall. but that's fine with me."; showrunner ruling B, 2026-11-08).**
   - A closing cell shrinks on its text fade clock (plan item 9).
   - Every frame draws its heading row. Body rows clip from the bottom. Clipped text is acceptable.
   - No frame draws a bordered box with no text. No frame draws a headless row.
   - At the last step the cell is heading only. Then it is gone.
   Test, red first: a fixture under 1 s in which an ended cell shrinks in mid-column, checked in the rendered buffer frame by frame. The fixture refuses an empty inner buffer (the first attempt's test allowed one at `render.rs:1020`). It also covers the frame where the roster has dropped the group while the grid still draws its closing cell (main 1fdee462: empty bordered box with `content rows: 0` in 38 of 107 frames; `frame_draw_group` returns without rows).

2. **The tile motion tests run under 1 s under load** (showrunner, 2026-11-07). Port's Mac run (six nextest threads, log `/tmp/claude/unit/a531f8d0-5a0c-483d-aa99-3738f735f9a7/mesh_runs/20261107T193135-3235537-9756.log`) timed seven `tui_pane` tile tests at 1.0 to 1.7 s; an earlier full run on the same code had a maximum of 0.645 s. In `tiles/grid/tests/motion_rules.rs`: `every_frame_of_a_key_keeps_the_motion_rules` (1.712 s). In `tiles/draw/tests/motion.rs`: `a_content_change_mid_motion_leaves_no_blank_row` (1.674 s), `a_cell_arriving_at_a_column_foot_leaves_no_line_before_its_first_row` (1.628 s), `a_content_change_keeps_the_motion_clock_with_a_key_waiting` (1.200 s), `sliding_pieces_leave_no_band_row_on_the_screen_ground` (1.015 s), `a_sliding_arrival_shows_its_nearest_content_row_with_its_first_visible_row` (1.180 s). In `tiles/draw/paint_tests.rs`: `startup_cells_leave_no_line_before_their_first_row` (1.383 s). A test near 1 s under load is a defect. Measure each test's stages, then cut the cost so each stays under 1 s in a loaded Mac full suite. Keep every assertion and every checked frame.

**Landing rule:** Phase 24's landing rule, unchanged. At most 3 repair rounds; the count restarts with the redesign (showrunner, 2026-11-08).

**Round 4, the last (showrunner, 2026-11-08):** the redesign capture said LAND no (`phases/31/diagnose5_report.md`). Closing-model faults fell to 0 (main 5104) and blank boxes to 22 (main 1058). One more round fixes two causes:

1. A new cell is admitted only while its command is Live. A Closing group that never had a cell does not take a slot (t40: seven jobs got a first cell after their work ended).
2. Admission counts reclaimable blank rows in live cells and closing spans, at O(C+W) and only when its inputs change (longest free wait t40 9.9 s against 2.45 s).

The next paired capture measures rule a against the approved closing model, and reports CPU as total reader CPU and time-weighted visible cells. Per-cell CPU is reported but does not decide landing. The free-wait measure stays as it is. If any live count is still worse, the showrunner decides land or park. No further round.

**First attempt (2026-11-07 to 11-08, copy head 9b85b0e, not landed):** it gated a closing cell's content all or nothing (lines of the unlanded 9b85b0e tree: `draw.rs:623`, `draw.rs:798`, `render.rs:1707`) while the grid still shrank its geometry. The paired capture found empty bordered boxes in 2387 frames against the base's 45. Keep its cost cuts: `draw_clipped` scratch reuse and the `GridLines::render` cut (`pane/frame.rs`), and the motion tests that run each key once.

**Files:**
- `crates/tui_pane/src/tiles/draw/{paint,contents,measurements,rows}.rs`, `crates/tui_pane/src/tiles/grid/{tile_grid,pieces,bands,heights,layout,allocation,rows,visual,arrangement,motion,selection}.rs`, `crates/cargo-tile/src/render.rs` — item 1 and its fixture: closing geometry, retained content, admission and painting. Tests: `draw/paint_tests.rs`, `draw/tests/motion.rs`, `grid/allocation_tests.rs`, `grid/arrangement_tests.rs` and `grid/tests/motion_rules.rs`. Item 2 adds each file that a moved defect names; the dispatch lists it with that defect's evidence.
- `crates/cargo-tile/src/render/frame_preparation.rs` — preparation inputs, retained tables, allocation decisions and row measurements.
- `crates/cargo-tile/src/render/command_cache.rs` — retained command widths, wrapped text, row cells and table layouts.
- `crates/cargo-tile/src/render/cell_paint.rs` — paint inputs, destination validation, command visibility and paint replay.
- `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/roster/invocation_continuity.rs` — the shared finish record and the canonical fade sample (state model; showrunner, 2026-11-08).
- `crates/tui_pane/src/pane/frame.rs` — the first attempt's cost cuts (`draw_clipped` scratch reuse, `GridLines::render`).
- `crates/cargo-tile/src/grid_view.rs` — expand and collapse apply the closing height after the viewport candidate.

**Workers:** 1 writer + 1 tester. `impl` owns source and regression fixtures; `test` owns paired captures.

**Constraints from prior phases:** Phase 28 did not land. These rules still hold on the current tree: Phase 25's Constraints, the exclusions from the rebuild sources, Phase 26's reader safeguards and Phase 27's CPU check. No new `#[allow]` or `#[expect]`; no test takes a second. `frame_draw_group` returns no content after its group leaves `VisibleRoster`, while the grid can retain a closing cell. The existing invocation snapshot keeps the ended rows, header and foot until that cell stops drawing. Include that retained content and its lifetime in preparation inputs; paint replay alone cannot supply missing content. Reader PTY tests and settled helpers do not prove animated leave, pause or retarget behavior. Use injected-time buffer fixtures and production-motion captures for those acceptance gates.

**Forward edits from Phase 46 (director, 2026-11-07):**
- `render.rs:524` returns the view to `Grid` when the expanded group leaves the visible roster. The grid can still paint a closing cell after that removal. Extend `an_ended_cell_returns_to_the_grid` to check the closing content, the focus, and the return during `Expanding` and `Collapsing`.

**Forward edits from Phase 49 (director, 2026-11-07):**
- Phase 49 footer contract: the summary foot is `SummaryFoot::WithOptionalSuffix` (`tui_pane/src/tiles/draw/foot.rs:17`), prefix `mem`, suffix `disk`. The unit is `G`. Memory has one decimal. Disk has one decimal below 10G and is whole from 10G. At a narrow width the foot keeps `mem` and drops the disk suffix and its gap. `FreeDiskSpace::Unavailable` (`census/scan/refresh.rs:143`) omits the disk value and its gap, with no user remedy.

**Acceptance gate:**
- The fixture fails before the fix and passes after.
- `verify.sh test` for `tui_pane`, `cargo-tile`, `cargo-handler` and `cargo-port`, green.
- The paired live capture against main meets the landing rule.
- Change-review line: an ended cell never paints as a headless row.

#### State model

Source: first-attempt copy `9b85b0e`; diagnosis `phases/31/diagnose4_report.md`. Written by the Phase 31 writer, 2026-11-08. Status: approved by the showrunner 2026-11-08, with the director's change: `HeadingOnly` is a time window on the fade clock, and no paint receipt exists.
Heading means the account/directory row produced by `CommandHeading`.
New types below are proposals. Their references identify current replacement sites, not existing definitions.

| Reference | File under `crates/` |
|---|---|
| R | `cargo-tile/src/roster.rs` |
| I | `cargo-tile/src/roster/invocation_continuity.rs` |
| O | `cargo-tile/src/roster/invocation_owners.rs` |
| C | `cargo-tile/src/render.rs` |
| P | `cargo-tile/src/render/frame_preparation.rs` |
| G | `tui_pane/src/tiles/grid.rs` at the unlanded 9b85b0e tree; today the motion owner is `tui_pane/src/tiles/grid/tile_grid.rs` |
| D | `tui_pane/src/tiles/draw.rs` at the unlanded 9b85b0e tree; today the painter is `tui_pane/src/tiles/draw/paint.rs` and the readout is `tui_pane/src/tiles/draw/rows.rs` |
| F | `tui_pane/src/pane/frame.rs` |
| V | `cargo-tile/src/grid_view.rs` |

Current sites of the named items, under `crates/tui_pane/src/tiles/`:

| Named item in the state model | Current site |
|---|---|
| `TileGrid`, admitted geometry, retained motion | `grid/tile_grid.rs:138` |
| `Transition.started` and `millis` | `grid/tile_grid.rs:192` |
| `LastPainted::Painted` | `grid/visual.rs:70` |
| `record_painted_body` | `grid/tile_grid.rs:2632` |
| `TilePiece` | `grid/pieces.rs:67` |
| `draw_placements` | `draw/paint.rs:96` |
| `has_visible_body_row` | `draw/paint.rs:219` |
| `draw_cell_with_rows_readout` | `draw/paint.rs:304` |
| `content_area` | `draw/rows.rs:347` |
| `a_resize_during_queued_closings_settles_focus_and_keeps_navigation_working` | `grid/arrangement_tests.rs:384` |
| `a_content_change_mid_motion_leaves_no_blank_row` | `draw/tests/motion.rs:391` |

**States and rows**

`ClosingCellFrame` borrows one source, one lifecycle sample, and one heading-bearing placement for the current frame.
It cannot represent a bordered placement without a heading row.
Only an admitted command cell creates closing geometry; an unshown command does not acquire a departure cell.

| State | Entry | Rows drawn | Exit |
|---|---|---|---|
| `Live` | The cell has a live invocation. | Existing heading and body. | All retained invocations finish; select the latest canonical finish as the close origin. |
| `FadingWhole` | Closing begins and its height holds the source. | Heading, reading, ancestry, table, process rows, and foot. | The shared fade reduces the available height below the source height. |
| `FadingClipped` | The available interior holds the heading and part of the body. | Heading and the top prefix of the same body. Bottom rows disappear first. | The interior reaches one row. |
| `HeadingOnly` | The fade sample reaches `A_h`, the start of the heading-only window. | Exactly the fitted account/directory heading; no table, progress, foot, or count. | The fade completes. |
| `Gone` | Closing completes after its terminal presentation. | No piece, cell border, readout, or summary fallback for that departure. | Release the presentation lease; never reopen from the same finished observation. |

| Boundary | Decision |
|---|---|
| The cell already has clipped content when it ends | Enter `FadingClipped`; retain its source and its actual starting height. |
| The viewport cannot contain a heading and its borders | Emit no piece or cell border. Retain the lifecycle source while its deadline runs; create no empty slot. |
| The same invocation becomes live again | The canonical lifecycle cancels closing. PID reuse keeps its distinct invocation identity. |

**One owner per fact**

Shared handles borrow the same record. They do not copy lifecycle facts into another mutable owner.

| Fact | Sole owner after redesign | Current site and change |
|---|---|---|
| Each invocation's end time and final process readings | `FinishedInvocation` inside the canonical `InvocationContinuity` | I:52, I:234. Share this finished record; do not clone its end timestamp into presentation rows. Keep `InvocationOwners`, O:19. |
| Current frame time | Existing shared `RosterClock` | R:76, R:795. Renderers and retained sources borrow it. |
| Fade duration and easing | `TilesConfig::fade()` plus the single lifetime curve exposed through new `CellLifecycleTiming` | `cargo-tile/src/config.rs:195`, R:311. The timing view borrows the finish owner and frame clock. Text and geometry consume one sample. |
| Close origin for a group | New `ClosingCommand` holds a handle to the latest finished invocation | Replace P:80. Store the owner reference, not another `Instant`. An earlier child finish does not close a live group. |
| Retained heading and body source | The canonical `TrackedGroup` and its invocation snapshot handles | R:339, I:16. Roster membership and the presentation lease share the source. Roster removal releases membership only. |
| Fitted heading text | One `CommandHeading` in the width-specific prepared command plan | C:7007, P:761. Share its account decision with the table. Refit on width or view changes; add no second formatter. |
| Prepared body rows | One width-specific prepared command plan | P:724, C:2559. It references canonical rows; it owns only derived wrapping and layout. |
| Starting height and closing geometry after roster removal | New `ClosingGeometry` owned by `TileGrid` | G:1451, G:1185. It has the admitted source height and band binding, but no start time or duration. |
| Last painted geometry | Existing `LastPainted::Painted` | G:1089, G:3942. This is previous-frame evidence, not a second current-height authority. Move the completed drawing into it. |
| Source release | New `ClosingCommand` presentation state | Replace clip-only retention at C:429 and G:3928. Source release depends on lifecycle completion only. |

| Today's duplicate or competing fact | Resolution |
|---|---|
| `VisibleRoster` copies `TrackedGroup` and its row lifecycle state, R:472, R:615. `ClosingCellSnapshot` clones it again, P:404. | Use selected source handles and derived filtered totals. Keep one canonical continuity; retain no copied finish time or `RowFade`. |
| Painted clones copy `RowFade`, R:370; closing snapshots can retain an old alpha. | Borrow canonical lifetime ink. Keep the existing shared entry decision and command-visibility callback; entry brightening does not restart closure. |
| Roster fade uses I:55/R:311; a later grid departure starts `Transition.started/millis`, G:1420. | The canonical lifetime sample drives closing text and height. Keep transition timing only for resident movement, entry, and viewport travel. |
| `PreparedTables.drawing`, P:77, and `PreparedFrame.drawing`, P:158, copy the drawn identity list. | Remove both lists. Retain sources by presentation leases, not copied geometry lists in table-preparation keys. |
| Grid and expanded preparations retain separate source copies, P:314/P:347. | Both plans borrow the same source. Their width-dependent layouts may differ; their lifecycle cannot differ. |

The shared canonical lifecycle boundary requires changes in R and I beyond the first attempt's drawing files.
The showrunner approved these scope additions (2026-11-08).

**Paths that construct or change records**

| Event and current path | State-model action |
|---|---|
| New scan: R:639 → R:657/O:40 → R:395 | Resolve the existing invocation key, then update its canonical source. Create no second lifecycle owner. |
| Job end: R:682 → R:420 → R:282 → I:234 | Finish once. Bind `ClosingCommand` to the latest finish owner and create `ClosingGeometry` from the last admitted placement. |
| Fade tick: `cargo-tile/src/terminal.rs:231` → R:784 | Sample `CellLifecycleTiming` once. Use its ordinary lifetime phase for heading ink and closing height; do not enqueue per-alpha resize steps. |
| Roster drop: R:786 | Release roster membership. Keep the same source through the presentation lease until `Gone`. Keep aliases while their canonical presentation owner remains. |
| Each grid motion step: G:2670 → G:3027 → G:3237/G:3792 | Resident steps change placement. Closing height still uses the existing lifecycle sample; the step queue cannot delay or restart it. |
| Resize: G:2335 → G:2458 | Refit the source for the new width and choose a heading-bearing band placement. Preserve close origin and progress. |
| Summary depth or row demand: G:2771/G:2807 → G:3027 | Move neighbors and reclaim released rows in the same sample. Never split a closing cell into a headless second piece. |
| Expand/collapse: V:107/V:121/V:137 → C:505 → G:3832 | Apply closing height after the viewport candidate. Pin the directory heading; scroll only the body. Viewport travel owns no departure clock. |
| Expanded group leaves the roster: C:551 → C:596 | Return to `Grid` as today. The grid borrows the same retained source and close phase for its remaining presentation. |
| Cached paint or scratch path: C:1919 → F:374 | Validate source generation, width/view, ink sample, and destination. Replay borrows the same heading-bearing frame; it owns no lifecycle decision. |
| Final removal: fade completion → G:3942 and presentation lease release | Erase the piece and release its rows together. Do not queue another shrinking step after the fade. |

**Height and clipping rule**

| Quantity | Rule |
|---|---|
| `H0` | The cell's admitted outer height when closing starts. |
| `a` | The ordinary lifetime fade sample from R:311. Use the same easing as the text. |
| `A` | The largest nonterminal fade sample; currently 254 with `FADE_ALPHA_STEP = 2`, `cargo-tile/src/constants.rs:176`. |
| Heading-only window | `A_h = floor(0.9 * A)`: the last 10% of the fade clock (300 ms at the default 3 s fade). |
| Closing outer height | `H(a) = 3 + floor((H0 - 3) * max(0, A_h - a) / A_h)` for `0 <= a <= A`. |
| Minimum | Three rows: top border, account/directory heading, bottom border. Shared borders count once in the lattice. |
| Heading only | `a >= A_h` gives `H = 3`, so the cell is heading only for the whole window. Fade completion is `Gone`. |
| Heading ink | Use the existing blend rounding toward source ink for nonterminal closing samples. Preserve a visible heading channel; start no entry-brightening state. |
| Height after retargeting | Cap the derived height by available space and the last committed closing height. Never restart from a larger height. |
| Enforcement | Construct new `HeadingPlacement` before a `TilePiece`, G:224/G:6847/G:6918. Never admit a one- or two-row closing rectangle. |
| Body clipping | Paint the heading at the first inner row. Copy only the top prefix of the prepared body below it. Keep the closing source shift at zero. |
| Foot | Include it at its source position. Clip it before earlier body rows; never reserve the final heading row for `content rows:`. Replace the closing use of D:879/D:1142. |
| Cross-column motion | Keep one closing piece per identity. Retarget its band atomically; do not paint old and new fragments independently. |


**Rules as frame checks — proposed changes only**

All checks use injected time and read the production draw buffer. This step runs none of them.

| Rule/state | Strict frame check | Existing test to extend |
|---|---|---|
| 1: all fading states | At 3 s and 30 s settings, text alpha and height match the same sampled phase. Closing starts before roster removal. | `render::expanded_view_tests::an_ended_cell_keeps_its_whole_content_through_every_leave_frame`, C:978 |
| 2: whole/clipped | First inner row equals the fitted directory heading. Subsequent rows equal the prepared body prefix; clipping removes only its bottom. | Same test, C:978; `render::tests::a_closing_cell_keeps_its_retained_ancestry_cut`, C:13214 |
| 3: every visible closing frame | Reject an empty inner buffer. Require a nonblank heading with visible ink. Reject any attributed border without that heading, including cached and shifted paths. | Same test, C:978; remove its empty-buffer escape at C:1020 |
| 4: heading only → gone | Observe exactly one interior heading row and no readout. The `HeadingOnly` window spans at least one frame at the default 3 s fade. After completion, find no piece, cell border, or repeated departure. | Same test, C:978 |
| All states after roster removal | Heading/source persist through grid return; focus settles correctly during `Expanding`, `Expanded`, and `Collapsing`. | `render::expanded_view_tests::an_ended_cell_returns_to_the_grid`, C:1051 |
| Resize/depth/motion | Preserve phase and minimum height; never produce a second headless fragment. Keep every existing checked frame. | `a_resize_during_queued_closings_settles_focus_and_keeps_navigation_working`, G:13180; `a_content_change_mid_motion_leaves_no_blank_row`, D:3641 |

The full-body assertion remains strict in `FadingWhole`.
The new clipped-state check compares an exact prefix; it does not accept arbitrary missing content.

**What goes and what stays**

| Remove or replace | Reason |
|---|---|
| D:623 gate, D:746 `VisibleCellBody`, D:785–800 whole-body predicate, C:1707 `minimum_visible_rows` | They hide text while retaining an enclosing slot. Replace them with heading-bearing geometry and bottom clipping. |
| P:703/P:725/P:935 minimum-complete-body calculations | The closing minimum is one heading row, not an entire process table. |
| Copied `ClosingCellSnapshot.group`, old-grid binding, `CellSourceLifetime`, and drawn-list cache inputs | Shared canonical source handles and explicit presentation lifetime replace these competing retention decisions. |
| Fade completion followed by a normal timed `Step` departure | Closing geometry already reaches its endpoint on the fade clock. |
| Per-height closing body relayout and bottom-pinned closing readouts | A width-specific source prefix keeps the heading and clips the bottom deterministically. |
| Keep F:345–400 scratch reuse and F:435 slice blits | These reduce moving-pane cost without owning lifecycle state. |
| Keep F:778–821 `GridLines::render` cut and D:718 background cut | These reduce traversal cost. They do not decide whether a closing slot exists. |
| Keep G:7367/G:7699 per-key motion fixtures and their assertions | Each key still runs once; the redesign adds no repeated fixture pass. |

Tests stay under 1 s each. The Phase 24 landing rule and the Phase 27 CPU check hold.

### Phase 32 — Phase 23's follow-ups: rows name what runs, and all arguments fit the screen  · status: done

#### As-built

A row for cargo running as the compiler wrapper names the compiler and its crate. With all arguments on, every process keeps its command row, and a cell or summary that cannot hold them shows `+N more`.

- `census/command_text.rs` parses a wrapper call into `CommandLayout::CompilerWrapper(CompilerCrate)`. The frontend file name comes from the first argument after cargo, and the crate comes from `--crate-name x` or `--crate-name=x`. A call with no crate name shows the frontend alone. With `h` on, the row shows the frontend and all arguments in argv order.
- The valued-option table in `constants.rs` owns nextest options and compiler options. `ValuedOptionScope::{GlobalAndRustc, CargoAndRustc}` bind `-C`, `-Z`, `--color` and `--target` to their values on wrapper rows.
- Row budgets are shared by measurement and painting. `reserve_command_rows` and `measured_table_row_use` return early only when every row is one line and arguments are Default. `HiddenProcessCountRow::{Absent, Reserved}` and `ProcessRowVisibility::{AllFit, SomeHidden}` name the count row and the overflow.
- `render/command_cache.rs` caches each command once per width, ground and table kind. `for_summary_row` reads the shared wrapped command. A summary command shows arguments only under the Phase 35m rule.
- `tui_pane` adds the public `HintToggleState` and the crate-private `HintValue`, both in `bar/status_line.rs`. `StatusLineGlobal::with_toggle` sets the on state, and the shortcuts surface shows it in plain words. `p processes` is the hint word.
- The tree key changes a short tile: a compact ancestor row shows, or a short hint says that the tree has no room.
- Ten render fixtures that assumed full capacity under arguments All use Default. Their assertions did not change.

**Files:**
- `crates/cargo-tile/src/census/command_text.rs`, `crates/cargo-tile/src/constants.rs` — the wrapper layout, the crate name, and valued options.
- `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/render/frame_preparation.rs`, `crates/cargo-tile/src/render/command_cache.rs` — row budgets, the count row and shared command caching.
- `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/app_global_action.rs` — toggle states and hint names.
- `crates/tui_pane/src/bar/status_line.rs`, `crates/tui_pane/src/bar/mod.rs`, `crates/tui_pane/src/lib.rs`, `crates/tui_pane/src/keymap/globals.rs`, `crates/tui_pane/src/overlays/global_shortcuts.rs` — the hint toggle state and its surface.

**Binds later work:** `capture_requested` in `hook.rs` reads only `ValuedOptionScope::Global`, so `--color`, `-Z` and `-C` need the repair in "The capture shim skips every option that cargo accepts before its subcommand". "All arguments keep the crate name, and status words match the key names" owns the crate name under `h` and the status-bar words. "No tile frame draws for more than 4 ms" owns the Mac run of this tree.

**Gotchas:** The count row takes one row under arguments All whenever a cell overflows, so a fixture that needs full capacity uses Default.

### Phase 32d — The capture shim skips every option that cargo accepts before its subcommand  · status: done

#### As-built

`ValuedOptionScope::accepted_before_subcommand(self) -> bool` is the one rule for the options that cargo accepts before a subcommand. It is a const exhaustive match, true for `Global` and `GlobalAndRustc`. `capture_requested` uses it, so `cargo --color auto metadata` reads `metadata` as the subcommand and requests no capture. The test `the_installed_shim_skips_every_valued_global_option` selects its options through the method and has explicit rows for `--color`, `-Z` and `-C`.

**Files:**
- `crates/cargo-tile/src/constants.rs` — the method.
- `crates/cargo-tile/src/hook.rs` — `capture_requested` and its test.

**Binds later work:** `accepted_before_subcommand` selects the options that any free argv subcommand reader skips.

**Gotchas:** `ArgumentOwner` in `census/command_text.rs` keeps its own table, because it decides more than this rule.

### Phase 32b — No tile frame draws for more than 4 ms  · status: done

#### As-built

`CARGO_TILE_FRAME_LOG` writes one line for each drawn frame, at no cost when the log is off. The line carries `cpu_ns` from `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`, `runq_ns` as context, stage CPU, and cache counters. `PreparedTables::prepare` keeps cell paintings across roster revisions and removes the paintings of departed groups. `CellPaintSource::matches` validates each painting against its `PaintedRowSource` before reuse. `FamilyPalette` caches family colors and follows theme role and reserved ink changes. An unchanged draw measures only the drawn layout.

The final capture has wall-clock draws over 4 ms of 65, 45 and 74 frames at 24, 40 and 80 rows, from 579, 632 and 720 on the parent tree. The 4 ms on-CPU bar is not met: 2 of 822, 1 of 752 and 17 of 730 frames exceed it, with a maximum of 5.723, 4.011 and 5.295 ms. The Mac run of `cargo-tile` is owed.

**Files:**
- `crates/cargo-tile/src/probe.rs` — the per-frame capture line and the thread CPU clock.
- `crates/cargo-tile/src/render/cell_paint.rs` — `PaintedRowSource`, `CellPaintSource` and retained paintings.
- `crates/cargo-tile/src/render/frame_preparation.rs` — paintings kept across roster revisions.
- `crates/cargo-tile/src/render/command_cache.rs` — source identity through `Rc::ptr_eq`.
- `crates/cargo-tile/src/render.rs` — the source check before reuse.
- `crates/cargo-tile/src/theme/mod.rs` — the `FamilyPalette` cache.
- `crates/tui_pane/src/runner/app.rs`, `crates/tui_pane/src/runner/event_loop.rs` — callback timing and the pending measurement before shutdown.
- `crates/tui_pane/src/tiles/draw/measurements.rs` — measurement of the drawn layout only.

**Binds later work:** A change to what a cell displays changes the sources that `CellPaintSource::matches` reads, or the cell keeps a stale painting. Run-queue wait is context and is never a gate value.

**Gotchas:** schedstat does not update per frame, so per-frame CPU reads `CLOCK_THREAD_CPUTIME_ID`. Each per-sample change in a `CargoProcess` or raw resource value rejects a painting, even when the displayed text is the same.

**Ruled out:** A wall-clock gate, because run-queue wait on a loaded machine is not draw cost.

### Phase 32e — The last tile frames over 4 ms of CPU  · status: done

**Depends on:** Phase 32b (it adds the per-frame CPU counters and the retained paintings that this phase measures).

#### Work Order

**Goal:** In the fixed busy capture, no `cargo-tile` frame spends more than 4 ms of on-CPU draw time. Phase 32b's final capture has 2 of 822, 1 of 752 and 17 of 730 frames over the limit at 24, 40 and 80 rows, with a maximum of 5.723, 4.011 and 5.295 ms. Sixteen of these frames spend their time in the draw callback, with paintings rejected. Four frames write about 16 kB.

**Spec:**

1. **Measure first.** For a slow frame that writes 19 bytes, record the first rejected source field with its old and new display values. Record preparation CPU apart from validation and repaint CPU. Record the repaint trigger beside the terminal diff cell count. Compare total draw time with and without the per-write clock reads.
2. **Validate paintings by what they display.** The source check compares the complete `CargoProcess` and raw resource values. Compare the values that determine the painting: the displayed duration and the formatted resource totals. Keep full precision in the accounting data. Aggregate contributions before rounding. Keep the layout, command visibility and destination checks.
3. **Remove the periodic full output.** The runner forces a repaint every two seconds, and unchanged cells then differ. Replace this with bounded row batches if periodic recovery is still necessary. Keep recovery for resize and external terminal damage. A recovery batch writes only in a frame that is already scheduled, the once-a-second status frame, so an idle tile draws no added frame. If periodic recovery stays, add an injected-time test: damage an idle terminal, and every row is correct within 2 s, the period of today's repaint.
4. **Buffer terminal output** before the counted stdout writer, if step 1 shows that small writes cost CPU.
5. **Compare with the same script on the final tree.** Run `python3 /tmp/claude/unit/02717738-2a97-4a2f-b362-e4add2f2dd8e/phases/32b/capture/cpu_build_capture.py <label>` from the home worktree, `/home/natepiano/rust/cargo-liner-tile`. It builds the release binary and runs `capture.py` with its `terminal_bridge.py`. It writes `<label>-results.json` in the same folder. Use `cpu5-results.json` there as the before result. Keep the tree unchanged during a capture. Report the before and after maximum, p99, count over 4 ms and sample count at each size.

**Files:**
- `crates/cargo-tile/src/render/{cell_paint,frame_preparation}.rs`, `crates/cargo-tile/src/render.rs` — the display signature and the retained paintings.
- `crates/cargo-tile/src/probe.rs` — the measurements of step 1.
- `crates/tui_pane/src/runner/{event_loop,constants}.rs` — the repaint trigger and the final draw measurement.
- `crates/cargo-tile/src/theme/mod.rs`, `crates/cargo-tile/src/theme/family_palette.rs` — the palette cache move and its test fixture.
- `crates/cargo-handler/src/`, `crates/cargo-port/src/` — consumer test gates only.

**Workers:** 2 writers
- `impl` — the render, probe and runner files, their tests, and final verification.
- `palette` — `theme/mod.rs`, `theme/family_palette.rs` and their tests (review items 4 to 6).

**Constraints from prior phases:** Phase 32b: `cpu_ns` reads `CLOCK_THREAD_CPUTIME_ID`, and run-queue wait is context only. Extend the retained-versus-fresh table in `cell_paint.rs` with subsecond clock changes and resource changes around rounding boundaries. No new `#[allow]` or `#[expect]`; no test takes a second. At most 3 repair rounds.

**Review items from Phase 32b, fixed in this phase:**
1. The disconnected-input return in `event_loop.rs` loses the final draw measurement.
2. The command-change case of `roster_revisions_keep_only_unchanged_cell_paintings` draws a different invocation. Change the command on the same invocation.
3. `PaintedRowSource::ended` stores a `bool`. Use a named completion enum.
4. `family_colors_follow_role_and_reserved_ink_changes` restores the theme without a `Drop` fixture.
5. The fallback expectation in that test duplicates `builtins::DARK_FAMILY_COLORS[0]`.
6. `FamilyPalette` sits in the `theme/mod.rs` module root. Move it to a child module.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`, then `... test cargo-tile` green.
- When `tui_pane` changes: `verify.sh lint tui_pane`, then `verify.sh test` for `tui_pane`, `cargo-handler` and `cargo-port`, green.
- The fixed capture shows no frame over 4 ms of `cpu_ns` at 24, 40 and 80 rows on the final release build. Phase 32b's final result stays beside it.
- A gate capture counts only when the 1-minute load is below 32 at its start and at its end. `capture.py` records both loads in the result. A capture outside the limit is void, not red. Do not wait idle for a low load: continue the repair round, and run the gate capture when the load is below the limit (showrunner, 2026-11-10).
- Change-Review: none — draw time only, with no visible change.

### Phase 32h — Tile frames stay under 4 ms after layout, view and value changes  · status: todo

**Depends on:** Phase 32e.

Phase 32e landed under the three-round rule with its gate open (finding F010). Its counted capture had on-CPU maximums of 3.24, 4.46 and 4.81 ms at 24, 40 and 80 rows, with 0 of 1720, 1 of 1668 and 13 of 1681 frames over 4 ms. Three interleaved pairs against main showed no change larger than the spread between pairs. The Astra review of F010 is `/tmp/claude/astra/20261110T183059Z-942485/review.md`.

#### Work Order

**Goal:** In the fixed busy capture, no `cargo-tile` frame spends more than 4 ms of on-CPU draw time at 24, 40 and 80 rows. This includes the first frame after a layout change, a view change, a duration change and a resource change.

**Spec:**

1. **Measure before you change.** The `capture_cpu_ns` timer surrounds `frame_draw_contents` (`crates/cargo-tile/src/render.rs:1728`), so it includes table preparation and text drawing. It also runs when `InkCapture::without_ink()` turns recording off. Measure foreground recording apart from ordinary painting. Keep source capture, destination scan, patch construction and ink binding as separate costs.
2. **Value changes count.** Frames over 4 ms also come after a duration change (`00:05` to `00:06`) and after a memory reading change, with `invalid_view=0`. `CellPaintSource::matches` checks layout before these source values (`crates/cargo-tile/src/render/cell_paint.rs:355`). Cover these frames, not only view changes.
3. **Make the recording areas small.** `paint_ink` saves and marks every foreground in its rectangle, then scans that rectangle again (`crates/cargo-tile/src/render/cell_paint.rs:591`). Ancestry recording gets the remaining pane (`crates/cargo-tile/src/render.rs:2443`): prepare the ancestry lines once, record only their drawn rectangle, and record nothing when there are no lines. Continuation recording gets the table area (`crates/cargo-tile/src/render.rs:2804`): resolve the continuation search first, then record only the selected row. Keep the full search dependency in `CellPaintReach`.
4. **Limits.** Do not skip recording only because the current fade is zero: a finished row can fade during later cache replays. Keep destination validation: continuation placement reads existing symbols, and retained patches use absolute buffer positions. Do not move the work into `before_draw`: that hides the cost from the draw measurement and does not reduce CPU work.
5. **Deterministic proof.** Add a fixture at 160 columns and 24, 40 and 80 rows. Make argument, hierarchy, duration and resource changes. Check the first frame after each change, not `settled_frame` (`crates/cargo-tile/src/render.rs:8754`), which draws four times. Compare each buffer with a fresh painting. Count foreground recording visits apart from the snapshot and patch counters. Empty ancestry needs zero recording visits. Continuation recording stays at one row as the table height grows. Include mixed live and finished rows, Brighten, wide glyphs and shifted buffers.
6. **The shifted-buffer test runs.** The loop at `crates/cargo-tile/src/render/cell_paint.rs:991` supplies only `Reset` and `Black`, so the shifted-buffer branch never runs. Add `Indexed(16)` to that loop.
7. **Constant order (finding F009).** Put `RECOVERY_BATCH_MILLIS` in `crates/tui_pane/src/runner/constants.rs` with the related repaint constants, in the module order that the style guide gives.

**Files:**
- `crates/cargo-tile/src/render.rs` — recording areas, the timer split and the first-frame fixture.
- `crates/cargo-tile/src/render/cell_paint.rs` — `paint_ink`, `CellPaintReach` and the shifted-buffer test.
- `crates/cargo-tile/src/render/frame_preparation.rs`, `crates/cargo-tile/src/render/command_cache.rs` — only if the measurement in item 1 names them.
- `crates/cargo-tile/src/probe.rs` — the separate recording costs.
- `crates/tui_pane/src/runner/constants.rs` — item 7.

**Workers:** 1 writer — `impl`, every file above.

**Constraints from prior phases:** Phase 32b: a change to what a cell displays changes its `CellPaintSource`, or the cell keeps a stale painting. Phase 32e: retained cells recolor from separate ink sources; keep them separate. Phase 32e: a frame outside the gate load limit is void, not red. No new `#[allow]` or `#[expect]`; no test takes a second. At most 3 repair rounds.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`, then `... test cargo-tile` green. When `tui_pane` changes: `verify.sh lint tui_pane`, then `verify.sh test` for `tui_pane`, `cargo-handler` and `cargo-port`, green.
- The fixed capture shows no frame over 4 ms of `cpu_ns` at 24, 40 and 80 rows on the final release build.
- A gate capture counts only when the 1-minute load is below 32 at its start and at its end. A capture outside the limit is void, not red.
- Three interleaved pairs against main for any CPU claim, with the median change and the spread.
- Change-Review: none — draw time only, with no visible change.

### Phase 32c — All arguments keep the crate name, and status words match the key names  · status: todo

**Depends on:** Phase 32 (it adds the compiler wrapper layout and the toggle state). Phase 32b measures the same files, so this phase starts after it.

#### Work Order

**Goal:** Two defects from Phase 32's design check (2026-11-09, `S/phases/32/review3_reply_ux_1.txt`) moved here under the hard landing. With `h` on, compiler rows in a narrow column read `clippy-driver ~/.rustup/toolchai…`, so every row is the same and no row names its crate. The status bar still says `tree` for the key named `processes`, and it draws the mode words in the version's ink.

**Spec:**

1. **A compiler row keeps its crate with all arguments on.** For `CommandLayout::CompilerWrapper`, the first row with `h` on has the same form as with `h` off: the frontend name and the crate value, with no `--crate-name` flag (`clippy-driver hana_diegetic`). A crate name wider than the command column is cut with `…` on that row. Every other argument follows in argv order on the argument rows, with the compiler path first. The `--crate-name` pair is not repeated. Wrapping, flag binding and the `…` rule do not change. A wrapper call with no `--crate-name` keeps argv order. Test, red first: add rows to `a_rustc_wrapper_row_shows_rustc_and_its_crate` that read the crate from the first row at the live command-column width (25 cells) and at 20 cells with `h` on. (Architect review 2026-11-09: `clippy-driver --crate-name hana_diegetic` needs 40 cells, so a reorder alone cannot keep the crate on the first row.)
2. **Status words use the key names.** `PROCESS_TREE_NOTE_LABEL` reads `processes`. The `processes` and `all arguments` notes use the bar's hint ink from `bar/support.rs`, not the version's ink. Other notes (`attract`, `frozen`) do not change. Test, red first: extend the status-line test that reads these notes from the buffer, for both text and ink.

**Files:**
- `crates/cargo-tile/src/census/command_text.rs` — item 1 and its parser cases.
- `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/render.rs` — item 2, and item 1's render test.
- `crates/tui_pane/src/bar/status_line.rs`, `crates/tui_pane/src/bar/support.rs` — item 2's note ink, only if the note style has no hint ink today. `crates/tui_pane/CHANGELOG.md` — one line if `tui_pane` changes.
- `crates/cargo-tile/CHANGELOG.md` — one Fixed line.

**Workers:** 1 writer — `impl`, every file above.

**Constraints from prior phases:** Phase 32b: `PreparedTables::prepare` keeps cell paintings across roster revisions, and `CellPaintSource::matches` validates each one against its sources. A change to what a command cell displays changes those sources, or the cell keeps a stale painting. Unchanged displayed values keep their reuse. This phase runs beside Phase 32e in its own worktree, so Phase 32e's capture tree stays unchanged. In `render.rs`, this phase edits only the status-line notes and item 1's render test; Phase 32e owns the rest. Phase 35m: a summary command shows its arguments only when the whole command fits one line, also with `h` on; otherwise it shows the named words. This phase does not change that rule. Phase 48 removed the `p` and `h` status-bar hints: the status bar keeps the mode words, and the on state of a key shows in the shortcuts surface. No new `#[allow]` or `#[expect]`; no test takes a second. At most 3 repair rounds.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`, then `... test cargo-tile` green.
- When `tui_pane` changes: `verify.sh lint tui_pane`, then `verify.sh test` for `tui_pane`, `cargo-handler` and `cargo-port`, green.
- Shots of a live tile with a `cargo clippy` whose children run as the compiler wrapper, with `h` off and on, at 24 and 40 rows; a fresh helper's design check passes.
- Change-review line: with all arguments on, a compiler row names its crate on its first row.
- Change-review line: the status bar says `processes` when the process tree is on.

### Phase 32f — Active crate labels read on a light background  · status: todo

**Depends on:** Phase 32c, Phase 32e

#### Work Order

**Goal:** The green active crate labels read on a white background. The Phase 32c design check measured them at 2.16:1 against white (showrunner, 2026-11-10). The dark-background look does not change.

**Spec:**

1. Find where the green comes from. Quote the source line that sets the ink for an active crate label.
2. If the ink comes from the `tui_pane` palette, fix it there, not in `cargo-tile`. If it comes from `cargo-tile`, fix it in `cargo-tile`.
3. If the fix is in `tui_pane`, read how `cargo-port` uses the same ink, and include `cargo-port` in the shots.
4. Change only what a light background needs. Every dark-background ink stays the same value. Test, red first: read the drawn ink of an active crate label from the buffer on a light and a dark background. The light ink has a contrast of at least 4.5:1 against the background, and the dark ink equals the current value.
5. Dimmed labels on a light background have a contrast of at least 3:1. The Phase 32c design check measured the column header `pid` at 2.01:1, the summary `pid` at 1.91:1 and the footer `content rows` at 1.96:1. Apply items 2 to 4 to the dimmed label ink, with the 3:1 limit.
6. Row text is visible on a light background. In the Phase 32c shots on a white terminal, the pid and command text of crate rows measured pure white on white, so the rows looked empty. The light theme in `crates/cargo-tile/src/theme/builtins.rs` already has black default text, and white default text belongs to the dark theme. So those shots probably show the dark theme on a white terminal: the capture set the terminal colors and selected no appearance.
7. Do this before any ink change, because the measurements in the Goal and items 5 and 6 came from those shots. Record the resolved theme and the terminal settings. Repeat the capture with matching light settings. If the measured defects go away, close the items they name and fix theme selection only if the theme was selected wrongly. If white text persists, read theme selection and the retained painting first. If palette values change, change the theme templates too: existing tests require matching constructors and templates.

**Files:**
- `crates/cargo-tile/src/render.rs` — the drawn-label tests and their buffer fixtures. `styled_process_command_words` assigns `success_color()` to crate labels.
- `crates/cargo-tile/src/theme/builtins.rs`, `crates/cargo-tile/themes/default_dark.toml`, `crates/cargo-tile/themes/default_light.toml`, `crates/cargo-tile/themes/high_contrast.toml`, `crates/cargo-tile/themes/starter.toml` — items 6 and 7, if palette values change.
- `crates/cargo-tile/src/theme/family_palette.rs`, `crates/cargo-tile/src/theme/mod.rs`, `crates/cargo-tile/src/render/cell_paint.rs` — when the ink is in `cargo-tile`.
- `crates/tui_pane/src/app_config/theme_install.rs`, `crates/tui_pane/src/theme/resolver.rs`, `crates/tui_pane/src/theme/state.rs` — only if item 7 reproduces a theme-selection fault.
- `crates/tui_pane/src/bar/palette.rs` — when the ink is in `tui_pane`. `crates/tui_pane/CHANGELOG.md` — one line if `tui_pane` changes.
- `crates/cargo-tile/CHANGELOG.md` — one Fixed line.

**Workers:** 1 writer — `impl`, every file above.

**Constraints from prior phases:** Phase 32b: a change to what a cell displays changes its `CellPaintSource`, or the cell keeps a stale painting. Phase 32e: retained cells recolor from separate ink sources; keep them separate. No new `#[allow]` or `#[expect]`; no test takes a second. At most 3 repair rounds.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`, then `... test cargo-tile` green.
- When `tui_pane` changes: `verify.sh lint tui_pane`, then `verify.sh test` for `tui_pane`, `cargo-handler` and `cargo-port`, green.
- Shots of a live tile with active crates on a light and a dark background; when `tui_pane` changes, shots of `cargo-port` with the same ink. A fresh helper's design check passes.
- Change-review line: active crate labels read on a light background.

### Phase 32g — The hidden-process row reads whole and matches its cell  · status: todo

**Depends on:** Phase 32c, Phase 32e

#### Work Order

**Goal:** Defects from the Phase 32c design check (2026-11-10) moved here under the hard landing. The row that counts hidden processes loses its last letter at 24 rows (`+18 mor`), and it draws as a full-width black bar with white text on a white cell. Items 4 to 7 name the other rows the check reported.

**Spec:**

1. The label `+N more` is whole at every count and every cell width that shows the row. Quote the source line in `draw_hidden_process_count` that cuts the label, and fix the cause there. Test, red first: add rows that read the whole label from the buffer at the 24-row cell width, with one-digit and two-digit counts.
2. The row draws on the background of its cell, in the normal row ink. It has no cut mark at its right end unless the label is cut. Test, red first: read the background and ink of the row from the buffer on a light and a dark background.

3. A cargo probe row keeps its real crate name: `clippy-driver ___`. It is the real value, and it marks the probe (showrunner, 2026-11-10). Make no change for this item. Tell the design check that this is the ruling.
4. A process row that fades after its process ends stays readable until the row goes. The Phase 32c design check measured a fading frontend name at 3.32:1 down to 1.60:1, on a terminal whose appearance was not recorded (Phase 32f item 7). Keep the existing removal time, and clamp the visible text contrast at 4.5:1 against its resolved background. Keep the retained ink handling of Phase 32e: `PaintedCellContents::recolor` reapplies `RowInk::color` from retained source colors, apart from fresh text drawing, so the clamp applies on both paths. Test, red first: read the drawn ink at intermediate fade samples on a light and a dark background. In the retained-versus-fresh fixture, the buffers are equal, the contrast is at least 4.5:1, and the replay does not repaint.
5. Check whether the path placement of a compiler call with no `--crate-name` is a defect. In the Phase 32c shots at 80 rows with `h` on, the path sits on the second row. `WrappedCommand::into_rows` combines continuation units into the last row when height is limited, so a row budget can change the placement. Compare the same argv and command width with only the row budget changed. If argv order and the available-space rules hold, close this item with no change.
6. With `h` off, a shortened crate name keeps its `…` directly after the name, at the solved command-column width. In the Phase 32c shot at 24 rows, the name `phase32_crate_00_with_a` stopped two cells early, and the only `…` was at the right margin. `mark_table_continuation` deletes existing ellipses when rows are omitted, then places one at the table margin. Keep both marks (showrunner, 2026-11-10): the name's own mark stays directly after the name, and the hidden-rows mark stands at the right margin. Stop the deletion for rows omitted. `reserve_command_rows` keeps returning 0 for single-line Default rows, so no process row is lost, and the Phase 35 rule stays. Test, red first: read a shortened final name and the margin mark from the buffer while more process rows stay hidden.
7. A flag and its value stay together on argument rows. In the Phase 32c shot at 80 rows, `-L` and `--out-dir` sat alone on a row, and `--check-cfg` ended a row while its value started the next. `COMMAND_VALUED_OPTIONS` omits these three options. Add them to the table, and reuse the existing `BoundValue` wrapping and cut behavior. Do not add a second classification in the renderer. These options are not lint options, so rename `ValuedOptionScope::RustcLint` and `ArgumentOwner::RustcLint` to `Rustc`, and `RUSTC_LINT_OWNERS` to `RUSTC_ARGUMENT_OWNERS`. Keep the Cargo and nextest boundaries. Test, red first: parser cases, and the argument rows of a recorded argv, for fitting pairs and for oversized values.

**Files:**
- `crates/cargo-tile/src/render.rs` — items 1, 2 and 4 to 6.
- `crates/cargo-tile/src/render/cell_paint.rs` — item 4's retained-versus-fresh fixture.
- `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/census/command_text.rs` — item 7 and its parser cases.
- `crates/cargo-tile/CHANGELOG.md` — one Fixed line.

**Workers:** 1 writer — `impl`, every file above.

**Constraints from prior phases:** Phase 32b: a change to what a cell displays changes its `CellPaintSource`, or the cell keeps a stale painting. Phase 32c: `compiler_argument_layout` identifies forwarded compiler executables, and `wrapped_process_command` reserves the heading row. A capture that sets terminal colors and selects no appearance can draw the dark theme on a white terminal. No new `#[allow]` or `#[expect]`; no test takes a second. At most 3 repair rounds.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`, then `... test cargo-tile` green.
- Shots of a live tile with hidden processes at 24 and 40 rows on a light background, and at 80 rows with `h` on with matched light and dark appearances. Record the resolved theme and the terminal settings before judging missing text, placement or contrast. A fresh helper's design check passes.
- Change-review line: the hidden-process row reads whole on the background of its cell.

### Phase 33 — A nextest run shows its tests, and CPU counts only what each process adds  · status: todo

**Depends on:** Phase 20, Phase 21, Phase 23, Phase 32, Phase 32c, Phase 32e (both change `runner/event_loop.rs`), Phase 43i

#### Work Order

**Goal:** A `cargo nextest run` started without a terminal draws its testing bar from its first line, and a command's CPU reading advances only by what its processes add, so a process that ended or went idle cannot hold it. From the user (2026-10-16, relayed by the showrunner): "why does catalyst-lfo not have a bar for building or testing … it says its cpu is 0% - why is it still showing?" The same phase makes a Mac test build of cargo-port free of a never-constructed variant, and brings the named slow reader and wire tests under a second, under load included. Files and lines below are as of phase 20's checkpoint and may move; find each by name.

**Spec:**

1. **A nextest run without a terminal shows its testing bar (F056).** `crates/cargo-tile/src/progress/capture_read.rs` reads a bar (`Counter::Bar`, phase from the line's status word, `Counter::phase` ~240) and a per-test tally (`Counter::Tally`, `(11/24)`). A nextest run without a terminal draws no bar and writes its first tally only after its first test finishes; the start line it prints first (`Starting N tests across M binaries`) is ignored, so the row shows no bar. Rule: the start line reads as `testing 0/N` (`Phase::Testing`, done 0, total N), and a later tally or bar replaces it as today. The README (`crates/cargo-tile/README.md` ~437–440: "A run whose tests are drawn without a terminal … reports the build alone, a test runner drawing no bar where nothing is watching") is stale: it states that a run without a terminal shows `testing` from its start line and then its tallies. A row that still shows at 0% CPU is intended: a command stays on screen while it runs, whatever its CPU, and the README says so in the same section. Tests, red first: `a_nextest_start_line_reads_as_testing_from_nought` (`parse_state` over a captured start line answers `testing(0, N)`); `a_tally_after_the_start_line_is_what_the_run_is_doing` (start line then tally answers the tally).
2. **CPU advances by each process's positive change (F057).** On Linux, `RetainedSubtreeCpuTime::update` (`crates/cargo-tile/src/census/invocation_cpu_accounting.rs` ~533-548, the rule at ~537) keeps `self.total = self.total.max(current.values().sum())`. When an orphaned mold helper leaves the tree, the sum falls below the retained maximum, and the reading carries that difference as a debt: CPU the live processes add does not move the total until it has made the debt up, so the row reads 0% while it works. Rule: the total advances by each process's positive change since the last scan, keyed by `ProcessIdentity`; a process that ended or went idle adds nothing and takes nothing away, so it cannot hold the reading. Linux moves an exited child's time into its reaper's counter (the field's comment, ~523), and that time is counted once. Follow the accounting path through `census/scan/census.rs`, where `Census::attribute_cpu_with` calls `InvocationCpuHistory::measure`. Tests, red first: `an_orphaned_helper_leaving_holds_no_cpu_debt` (a helper's time leaves the tree, then live processes add CPU: the reading rises by what they added); `cpu_advances_by_each_processs_positive_change`; `a_reaped_childs_time_is_counted_once`. **The CPU oracle changes with the rule.** Phase 27's oracle mirrors the current rule: `NativeCpuHistory.sample` (`crates/cargo-tile/src/shim_registration/reader_scenario_cases.py`, the `CounterRetention.REAPED_CHILDREN` branch) keeps `max(total, sum(counters))`, and `NativeCpuOracle.observe` asserts the product's `accumulated` within 1e-7 of the oracle's. The same change moves that branch to the new per-process rule; no other oracle check changes and no assertion is relaxed. Test, red first: `reader_cpu_oracle_follows_an_orphan_departure_and_a_reaped_child` (supplied counters: the orphan-departure and reaped-child sequences of the three tests above, through the oracle, give the values those tests assert for the product).
3. **Already satisfied.** `PlatformCreationTokenUnavailable` and its constructor have matching platform conditions; no source change is required.
4. **The named slow tests run in under a second** (showrunner: logged for this phase's reader work). These cargo-tile tests took one second or more in phase 20's suite runs, all of which passed; each runs in under a second.
   - Mac:
     - `shim_registration::reader_scenarios::reader_keeps_parent_family_across_its_only_childs_source_switch` — 1.746 s
     - `shim_registration::reader_scenarios::reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` — 1.878 s
     - `shim_registration::reader_scenarios::reader_excludes_a_live_command_without_sweeping_its_capture` — 1.452 s
     - `shim_registration::reader_scenarios::reader_ignores_foreign_owned_account_and_reports_its_owner_in_settings` — 1.340 s
     - `shim_registration::reader_scenarios::reader_keeps_one_direct_row_for_quiet_json` — 1.648 s
     - `shim_registration::reader_scenarios::reader_opens_settings_when_resize_and_key_are_pending_together` — 1.151 s
     - `hook::tests::mutations_acquire_the_lock_before_classifying_an_orphaned_shim` — 1.048 s
     - `hook::tests::mutations_acquire_the_lock_before_inspecting_unreadable_cargo` — 1.177 s
     - `hook::tests::removing_with_a_held_lock_leaves_the_toolchain_untouched` — 1.136 s
     - `progress::capture::tests::taking_two_resolved_roots_shares_one_removal_allowance` — 1.161 s
     - `progress::capture::tests::unknown_staging_inspections_leave_allowance_for_ended_pairs_across_roots` — 1.070 s
     - `progress::capture::tests::incomplete_registration_inventory_sweeps_any_sampled_proven_pair` — 1.891 s
     - `shim_registration::shared_capture::account_child_has_account_environment_and_only_machine_report_lines` — 1.024 s
     - `shim_registration::wire::birth_stamp_belongs_to_the_pid_in_the_registration_filename` — 1.005 s
     - `shim_registration::wire::json_arguments_preserve_both_quiet_spellings_after_separator` — 1.408 s
     - `shim_registration::wire::matching_relative_directories_keep_their_absolute_paths_and_writer_homes` — 1.246 s
     - `shim_registration::wire::same_pid_and_calendar_second_receive_different_generations` — 1.202 s
   - natedev:
     - `shim_registration::reader_scenarios::reader_keeps_parent_family_across_its_only_childs_source_switch` — 1.073 s
     - `shim_registration::reader_scenarios::reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` — 1.116 s
     - `shim_registration::shared_capture::staged_executable_installs_toolchains_and_cleans_up` — 1.074 s
   - Reported by the showrunner (2026-10-18; next-items "Residual timing of three cargo-tile tests"): `reader_keeps_parent_family_across_its_only_childs_source_switch` (1.24 s), `render::tests::a_command_cell_leaves_no_spare_row_while_a_column_mate_is_short` (1.17 s) and `staged_executable_installs_toolchains_and_cleans_up` (2.82 s).

   These times are older than Phase 26. First time each test on the current tree, alone, on the machine that reported it; a test under 1 s there is recorded and closes. Measure each remaining test's cost before cutting it, and time each cut on the machine that reported it (memory `attempts-reader-tests-mac-under-a-second`: measure each cut on the Mac in turn with the version before it). Phase 26's repairs stay: the staged reader copy on macOS and the reader readiness waits.
5. **The compiler cache reader test: remaining run time and completion under load.** Phase 26 replaced the floor of 10 that failed under load (natedev, 2026-10-16, load 75 to 85: compile CPU 7 in every sample, 1.784 s) with a floor of 1, required the sustained samples to span a reporting window (`cpu_observation_is_complete`), and stopped the Mac reader from scanning the build artifact folder at startup (a staged reader copy). Phase 26 timed the test at 0.955 s alone and 1.31 s in the full suite; earlier full runs took 1.390 s to 4.4 s (Phase 32's add-on, moved here). Phase 27 then landed the strict CPU oracle, and the test became slower: 7.5 s in a focused Mac run and 15.439 s in the full Mac suite at a 1-minute load of 27 to 34. After the strict replay, `stopped_compiler_cpu` (`reader_scenario_cases.py`) stops the compiler and waits for two later scan records. Rule: on the current tree, measure startup, CPU observation, the kernel anchor and cleanup separately, alone and in the full suite, before any cut; cut any wait on wall clock or load at its cause. "Phase 27's check unchanged" means all of these stay: integer-nanosecond scan and publication times, the smoothing and hold replay, unique process contributions, the exact charged pid set, the independent stopped-compiler kernel anchor, both client-pid readiness waits before the reader is released, and the PTY close before `waitpid` on a direct SIGKILL. Item 2's change to the oracle's retention branch is the one permitted oracle edit. Test: the test passes repeated under a 1-minute load at or above 75 on natedev, in under a second, with Phase 27's check unchanged and the five `reader_cpu_oracle_*` tests green.

6. **The bare `cargo-nextest` binary shows test progress** (showrunner, 2026-10-16, from what the user saw). A `verify.sh` Mac offload test run showed build progress 0→100% in cargo-tile, then no test progress, though natedev runs of the same suite show it. On the Mac the step runs as user `hana-ci`, as the bare binary: `/run/current-system/sw/bin/cargo-nextest nextest run --profile ci --all-features -p hana …`, not `cargo nextest`. Rule: a nextest run reads its testing progress whatever form starts it: `cargo nextest run`, or the `cargo-nextest` binary run directly with `nextest run`, from any path and under another account. Find where an invocation is recognized as a nextest run and where its capture is attached (by name; for example `census/command_text.rs` and the capture registration), first reproduce it on the current tree with a live Mac offload run. Phase 50 recorded an old reader, and HEAD already covers a captured `cargo-nextest nextest run` (`render.rs` `a_captured_ci_nextest_draws_its_meter_without_lending_it_to_a_child`). Repair only a reproduced fault, at the step that drops it; with no reproduction, record the run and close the item. Item 1 stays: `counter_at` still has no `Starting N tests` case. **Test, red first, for a reproduced fault,** named for the behavior (for example `a_bare_cargo_nextest_binary_run_shows_its_tests`).
7. **Each app shows its own CPU beside its uptime.** From the user (2026-10-16, relayed by the showrunner): "add a tile-fix to output the cpu % next to uptime across all cargo-liner binaries including cargo-port to show the cpu% that the binaries own process is taking to we can tell how heavy weight they are. you can choose the best way to represent this yourself." Showrunner's design calls:
   - **Where:** `tui_pane`'s framework-owned status-line segment, beside `uptime_secs` in `bar/status_line.rs`, so cargo-port, cargo-tile and cargo-handler all get it from one place and no binary carries its own copy.
   - **What it shows:** `CPU: 2.4%` right after the uptime, with the same label and value styling as `Uptime:`.
   - **How it is measured:** the process's own CPU as a percent of one core, the way top counts it, so one spinning thread reads 100%. One decimal under 10%, whole numbers at 10% and above. Take it from `getrusage(RUSAGE_SELF)`: the change in user plus system time divided by the change in wall time since the last sample. This works the same on Linux and macOS.
   - **When it updates:** no shared uptime tick exists today: `StatusLine` is rebuilt for each draw, each binary supplies `uptime_secs`, cargo-handler redraws by the second only while agents are displayed, and cargo-tile repaints for its own causes. The framework owns one persistent sampler, kept across frames (`crates/tui_pane/src/framework/`), and the runner schedules one status-line visual deadline at the uptime's next whole second (`crates/tui_pane/src/runner/deadline.rs`), the wakeup a live uptime needs anyway; no second polling loop exists. The sample is taken on that tick, so a reading covers the second since the last one. Idle: that once-a-second status-line frame is the only frame an otherwise idle app draws. Paused (Phase 34): pause holds the cells, and the status line keeps its uptime and CPU current. The app call sites that build `StatusLine` do not change.
   - **Before two samples exist:** a type, not a guard, for example `CpuReading::{Pending, Measured(..)}`, with `Pending` drawn as `CPU: —`.
   - **Tests:** no test runs at real speed. Drive the sampler with injected clock and rusage values; test the formatting at 9.9% and 10%, and `Pending`. Cadence, with an injected clock: `the_status_deadline_falls_on_each_uptime_second` (the runner's deadline lands on each whole uptime second and nothing else), and `an_idle_app_draws_one_status_frame_a_second`.

**Files:**
- `crates/cargo-tile/src/progress/capture_read.rs` — item 1's start line and its tests.
- `crates/cargo-tile/src/census/invocation_cpu_accounting.rs`, `crates/cargo-tile/src/census/scan/census.rs`, `crates/cargo-tile/src/census/scan/census_tests.rs` — item 2 and its tests, and any other census file the accounting leads to.
- `crates/cargo-tile/README.md` — item 1's stale lines and the 0% CPU sentence.
- `crates/cargo-port/src/process_observation/identity.rs` — item 3.
- `crates/cargo-tile/src/shim_registration/reader_scenario_cases.py`, `crates/cargo-tile/src/shim_registration/reader_scenarios.rs` — item 2's oracle retention branch and its test, and items 4 and 5's reader tests. `reader_scenario.py` stays the launch wrapper; CPU accounting changes go in `reader_scenario_cases.py`.
- `crates/cargo-tile/src/shim_registration/` — item 4's wire and shared-capture tests (`wire.rs`, `shared_capture.rs`).
- `crates/cargo-tile/src/hook.rs`, `crates/cargo-tile/src/progress/capture.rs` — item 4's `hook::tests` and `progress::capture::tests` only.
- `crates/cargo-tile/src/render.rs` — item 4's `render::tests::a_command_cell_leaves_no_spare_row_while_a_column_mate_is_short` only.
- `crates/cargo-tile/src/census/command_text.rs` — item 6 and its test; if the live Mac measurement places the cause in another `cargo-tile` file, the repair goes there and the phase summary names that file.
- `crates/tui_pane/src/bar/status_line.rs` — item 7 and its tests, with the CPU sampler in its own file beside it in `bar/`.
- `crates/tui_pane/src/framework/mod.rs`, `crates/tui_pane/src/runner/deadline.rs`, `crates/tui_pane/src/runner/event_loop.rs` — item 7's persistent sampler and its once-a-second status-line deadline, and their cadence tests.
- `crates/cargo-tile/CHANGELOG.md` — one Fixed line each for items 1, 2 and 6; `crates/tui_pane/CHANGELOG.md` — one Added line for item 7.

**Workers:** 2 writers
- `impl` — items 1, 2, 3 and 6, item 5's CPU-oracle checks, cargo-tile source, README, the cargo-tile changelog and their tests. Items 4 and 5 measure current timings. Phase 43j owns its unresolved timing defects. Serialize shared reader-fixture edits with Phase 43j. This phase repairs only timing regressions caused by its changes.
- `status` — item 7: `tui_pane` status rendering, the CPU sampler, the framework, runner deadlines, the `tui_pane` changelog and their tests.

**Constraints from prior phases:** Phase 32b: `PreparedTables::prepare` keeps cell paintings across roster revisions, and `CellPaintSource::matches` validates each one against its sources. A change to a command or resource value that a cell displays changes those sources, or the cell keeps a stale painting. Unchanged displayed values keep their reuse. Phase 20's rules hold: each reading is drawn once, beside its own row, and a row's reading is derived from `InvocationResourceUse` values (Phase 20 item 15). Phase 43b: each scan prepares one `Census` and one CPU topology, and on the Mac `InvocationLifetimeReads::apply` patches CPU and lifetimes on that one census; item 2's accounting reads that census and adds no second one. Phase 17 shortened reader tests by overlapping the test scaffolding with the real reader's startup; the measured floor then was the reader's own startup (release to first complete frame 0.51 s, to the CPU rows 0.44 s more). Phase 26 later stopped the Mac reader's startup scan of the build artifact folder, so that floor is old evidence: measure it again. Phase 27's CPU check holds as item 5 defines it; items 4 and 5 change no CPU assertion, and item 2 changes only the oracle's retention branch. Phase 27's test-only recorder (`census/cpu_observation.rs` and its `#[cfg(test)]` hooks in `census/scan/census.rs`, `census/scan/refresh.rs`, `census/invocation_cpu_accounting.rs`, `terminal.rs` and `app.rs`) stays; item 2's accounting change keeps `cpu_observation::work` before accounting consumes the counters. Phase 21: `TrackedGroup::without_hidden_descendants` (`crates/cargo-tile/src/roster.rs` ~256) re-sums a lead's subtree after a hidden nested subtree leaves, through `scan::subtree_resource_totals` (`pub(crate)`, `census/scan/invocation.rs` ~759), which sums each row's own `resource_use`; item 2's accounting change lands in each row's own `resource_use`, so the census and the roster re-sum agree. `CommandText::subcommand` (`census/command_text.rs` ~338) reads the retained subcommand from parsed `CommandWord` values, whose option arity comes from Phase 23's one table `COMMAND_VALUED_OPTIONS` (`constants.rs` ~509, each entry tagged by `ValuedOptionScope`; Phase 32 added `GlobalAndRustc` and `CargoAndRustc`, and Phase 32d's `accepted_before_subcommand` is the one rule for options before a subcommand), and `CommandText::is_hidden` reads it; The required rule makes the free argv `subcommand` that `commands.excluded` uses and the shim's rendered skip list read the options that `accepted_before_subcommand` selects from that same table. Item 6's repair keeps `is_hidden` answering `nextest` for the bare `cargo-nextest nextest run` spelling as for `cargo nextest run`. The dependencies name completed prerequisites. Phases 22, 24, 25 and 28 do not land. Phase 34 starts after Phase 33; one writer owns shared source and documentation at a time. A `tui_pane` change also runs `verify.sh test` for `cargo-handler` and `cargo-port`. No new `#[allow]` or `#[expect]`; no test takes a second. Keep capture-progress changes in `RosterRevision`. Keep status CPU and uptime deadlines outside table-preparation inputs. Preserve the accepted-scan stamp on each drawn frame. Phase 43h ("Mac reader tests stay under 1 s under load") keeps the compiler stopped until two later scan records complete, then reads the independent kernel CPU counter; keep that order, the strict counter bound and the `SIGCONT` cleanup. Complete nested-command readiness checks before the reader barrier release, with every readiness assertion. General fixture shortening belongs to Phase 43j for its named tests. Preserve its accepted fixture changes. This phase repairs only timing regressions caused by its own changes; the final suite measurement belongs to "Slow tests start first".

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile`, `... lint cargo-port` and `... lint tui_pane` once, then `... test cargo-tile` and `... test cargo-port` green; items 1 and 2's tests red before their fix and green after; item 7's cadence tests green; no test takes a second or more.
- Every test item 4 names under a second on the machine that reported it; item 5's test green under a 1-minute load at or above 75 on natedev.
- A Mac test build of cargo-port (`ssh mac`) reports no never-constructed variant.
- Item 7: `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler` and `... test cargo-port` green; live shots of cargo-port, cargo-tile and cargo-handler each show `CPU:` beside the uptime.
- Change-review line: the CPU % shows beside uptime in all three apps.
- Live Mac offload run (a `verify.sh` test run offloaded to the Mac, which runs as `hana-ci` through the bare `cargo-nextest` binary): cargo-tile shows its testing progress after the build reaches 100%; confirmed in the checkpoint.
- Change-review lines (production doc's change review file), one per visible change: a nextest run without a terminal shows `testing` from its first line; a command's CPU reads what its live processes add after a helper ends.

### Phase 34 — Space pauses the display  · status: todo

**Depends on:** Phase 20, Phase 21, Phase 23, Phase 32, Phase 33

#### Work Order

**Goal:** The space bar pauses and unpauses cargo-tile while it runs, and the status bar says `paused` the whole time it is paused. From the user (2026-10-16, relayed by the showrunner): "make it so the space bar pauses and unpauses cargo tile while it's running (and it should show paused in the statusbar while that's happening)". Pausing is the display freeze cargo-tile already has: the screen holds still, and cargo keeps running underneath. Today that freeze is on `f` and the status bar says `frozen`. This phase moves it to space and calls it pause everywhere. It runs after Phases 21 to 33: Phase 33 and this phase both change the render files, so one writer owns them at a time. Files and lines below are as of phase 20's checkpoint and may move; find each by name.

**Spec:**

1. **One key, one name.** The default binding moves from `f` to `space`, and `f` is left unbound. The action is renamed from freeze to pause throughout, with no alias for the old name. Neither machine's cargo-tile config names `global.freeze` (checked 2026-10-16). The renames:
   - the action: `AppGlobalAction::Freeze` ("freeze", "Freeze the display") becomes pause, "Pause the display" (`crates/cargo-tile/src/app_global_action.rs` ~43, ~69, ~93);
   - the default keymap line: `global.freeze = f` becomes `global.pause = space` (`crates/cargo-tile/src/keymap.rs` ~215);
   - the state: `Updates::Frozen` becomes `Updates::Paused`. `Updates` is public `tui_pane` API (`crates/tui_pane/src/attract/updates.rs`, exported from `attract/mod.rs` and `lib.rs`), so the rename reaches every consumer: `toggled` and the doc comments in `updates.rs`; `Attract::advance` (`crates/tui_pane/src/attract/controller.rs` ~1031-1055, which holds the attract strip while paused) and its tests `frozen_frame_records_its_laid_out_area` and `frozen_frame_does_not_change_band_glyphs`; the favorites overlay test `an_adjusted_random_favorite_schedules_a_lowercase_warning_while_frozen` (`crates/tui_pane/src/favorites_overlay/globals.rs` ~403); and in cargo-tile `render.rs` ~5988, `app_global_action.rs` ~207, ~228 and ~244, and `terminal.rs` ~194. cargo-handler uses only `Updates::Live` (`render/mod.rs`, `render/accounts_grid_tests.rs`) and needs no edit. The three `tui_pane` tests take the `paused` name and keep their assertions unchanged, as the regression that the held attract strip and the favorites warning still behave the same;
   - the status note: `FROZEN_NOTE_LABEL` (`"frozen"`, `crates/cargo-tile/src/constants.rs` ~364) becomes a `paused` note, drawn through `StatusLineNote::flag` as today (`crates/cargo-tile/src/render.rs` ~6060).
2. **Behaviour unchanged.** While paused, what the freeze skips today stays skipped (`crates/cargo-tile/src/terminal.rs` ~188: scans are drained and discarded, fades and travel hold). The first frame after unpausing describes the world as it is then. Apart from item 6's held motion clock, only the key and the words change.
3. **Where space already means something.** The settings overlay steps a setting on space (`crates/tui_pane/src/runner/keys.rs` ~148); an open overlay keeps its own keys, and pause applies when the grid has the keyboard, as the freeze does today. The attract screen's steering keys take no space (README ~628 says the grid's letters stay free there; keep that true for space and say so). The keymap overlay's capture, which binds the next key pressed, still captures space as a key.
4. **Words.** Every user-facing mention of freeze or `f` for this action becomes pause and `space`: README (~628 and any other), the `?` overlay description, doc comments that name `f` (`crates/cargo-tile/src/app.rs` ~331, `terminal.rs` ~188 and ~262).
5. **Tests, red first:**
   - `space_pauses_and_unpauses_the_display`: a running app, space, and the drawn status line reads `paused`; space again, and the note is gone and scans apply again. It reads the rendered status line back.
   - `the_default_keymap_binds_pause_to_space`: the generated default keymap file binds `global.pause = space` and nothing binds `f`.
6. **Pause holds the motion clock, not only the updates.** Skipping scans does not hold travel: `TileGrid::progress` (`crates/tui_pane/src/tiles/grid/tile_grid.rs:2520`; `MotionClock` is in `grid/motion.rs:27`) reads `transition.started.elapsed()` on whatever frame draws, and Phase 33's once-a-second status-line frame draws while paused, so a paused grid would advance its motion and fades on every status tick. Rule: this phase owns freezing the clock that cells read. While paused, every frame draws each cell's geometry and its text inks exactly as on the frame pause began; the status line alone moves. On unpause, motion and fades continue from where they stood, as if the paused span had not passed, with no jump. The grid takes a paused state through an additive `tui_pane` method (for example `TileGrid::pause(at)` / `resume(at)`, or a clock the grid reads that pause stops) that shifts elapsed motion by the paused span; the row fades cargo-tile computes read the same held clock. Phase 36 replaces the step queue with `step(displayed, target, dt)`, and carries this as a paused `dt` of zero. Test, red first, with injected time: `a_paused_grid_draws_the_same_cells_across_status_ticks`: a grid mid-motion with a fading row, paused, then several status ticks; every frame's cell area matches the paused frame cell for cell; after unpause the next frame continues from the paused progress, not from wall time.

**Files:**
- `crates/cargo-tile/src/grid_view.rs` — the view clock that `Opening`, `Expanding` and `Collapsing` keep.
- `crates/cargo-tile/src/app_global_action.rs`, `crates/cargo-tile/src/keymap.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/terminal.rs`, `crates/cargo-tile/src/app.rs` — items 1 to 3 and the renames.
- `crates/cargo-tile/src/render.rs` — the note line, item 5's render test, and item 6's row fades on the held clock with its test.
- `crates/cargo-tile/src/render/frame_preparation.rs` — preparation inputs, retained tables, allocation decisions and row measurements.
- `crates/cargo-tile/src/render/command_cache.rs` — retained command widths, wrapped text, row cells and table layouts.
- `crates/cargo-tile/src/render/cell_paint.rs` — paint inputs, destination validation, command visibility and paint replay.
- `crates/tui_pane/src/tiles/grid/tile_grid.rs`, `crates/tui_pane/src/tiles/grid/motion.rs`, `crates/tui_pane/src/tiles/grid/tests/motion.rs`, `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — item 6's paused motion clock and any additive export it needs, and its grid test.
- `crates/tui_pane/src/attract/updates.rs`, `crates/tui_pane/src/attract/controller.rs`, `crates/tui_pane/src/favorites_overlay/globals.rs` — item 1's `Updates::Paused` rename, its consumers and their three tests.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md` — item 4, and one Changed line; `crates/tui_pane/CHANGELOG.md` — one Added line for item 6 and one Changed line for the `Updates::Paused` rename.

**Workers:** 1 writer: a rebinding, a rename and the pause clock it needs, one path from the key to the held frame. The `Updates` rename crosses `tui_pane` and cargo-tile and stays with this one writer.
- `impl` — every file above, items 1 to 6, the whole rename and the pause path.

**Constraints from prior phases:** space already parses as a key (`key_bind.rs` ~349). This phase makes two `tui_pane` changes: item 6's paused motion clock, which is additive, and item 1's rename of the public `Updates::Frozen` variant to `Updates::Paused`, which changes public API and gets its Changed line. No other existing `tui_pane` public signature changes. A `tui_pane` change also runs `verify.sh test` for `cargo-handler` and `cargo-port`. Phase 35 later reworks `render.rs`'s table walk; this phase touches only the status note there. Phase 34 starts after Phase 33; one writer owns shared source and documentation at a time. Phase 33 adds a framework-owned `CPU:` segment and a once-a-second status-line tick; pause holds the cells, and the status line keeps its uptime and CPU current while paused. No new `#[allow]` or `#[expect]`; no test takes a second. Rename the app-local `Updates::Frozen` variant when `attract` is disabled. Include pause state and held cell time in preparation. Keep status time live. Reader PTY tests and settled helpers do not prove animated leave, pause or retarget behavior. Use injected-time buffer fixtures and production-motion captures for those acceptance gates.

**Forward edits from Phase 46 (director, 2026-11-07):**
- Pause holds the `GridView` clock (`grid_view.rs:86`) as well as the grid clock. Extend the injected-time test across `Opening`, `Expanding`, `Expanded` and `Collapsing`, with status-only frames and resume without a jump.
- `Opening` stays private to key routing and preparation. Keep `quit_after_enter_before_the_first_paint_quits_the_app` green; add no screen label.

**Forward edits from Phase 49 (director, 2026-11-07):**
- `drain_scans` (`terminal.rs:297`) stores the disk value. Extend `a_paused_grid_draws_the_same_cells_across_status_ticks` with available and unavailable disk scans during status ticks, in Grid and Expanded views. Assert an unchanged foot while paused, and the next accepted disk value after resume.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile` and `... lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler` and `... test cargo-port` green; the three tests red before the change and green after; no test takes a second or more.
- No `Updates::Frozen` remains (`rg 'Updates::Frozen' crates` is empty); the three renamed `tui_pane` tests of item 1 pass with their assertions unchanged.
- Unit director's live capture: a running tile, space, and the status bar reads `paused` while the cells hold still with a build running; space again, the note leaves and the cells update.
- Change-review line (production doc's change review file): space pauses and unpauses the display, and the status bar says `paused`.

### Phase 35 — Phase 20's follow-ups: one table walk, plain caches, and whole marks  · status: todo

**Sort (2026-11-06):** items 4, 20, 22, 24, 29, 30, 36, 37 and 39 are dropped (user, 2026-11-06: 'i don't want these style changes i didn't ask for'). Item 2 was met. Items 1, 3, 11, 13, 15, 16, 27, 31 and 32 stay (showrunner, 2026-11-06: the user's rule covers visible changes only). Items 5 to 10, 12, 14, 17 to 19, 21, 23, 25, 26, 28, 33, 34, 35 and 38 stay: each fixes a wrong, clipped or unreadable result.

**Depends on:** Phase 20, Phase 21, Phase 23, Phase 32, Phase 33, Phase 34

#### Work Order

**Goal:** The items phase 20's repair rounds moved out under the production's hard-landing rule land here. From the third repair round on, a finding stays in its phase only if it is a regression the phase caused, breaks something that worked, or fails a test or gate; these are none of those. A table's hidden-rows mark is drawn by one path and decided on the frame that paints it; the frame's caches key on every input that changes their answer and look up without a scan; one walk answers a table's header and remaining rows; a profile label and a Long ancestry cut are whole or marked at the cut; and the code, tests and changelog line phase 20 left follow the crate's conventions. Files and lines below are as of phase 20's repair round 3 and may move; find each by name.

**Spec:**

1. **One path draws the hidden-rows mark** (phase 20 finding F029, nit, moved here open; close it with this item) (`crates/cargo-tile/src/render.rs` ~3605). `TableContinuation::RowsWereOmitted` can be drawn by two paths, so where a table's mark lands depends on which path drew the table. Rule: one path draws the hidden-rows mark, in both views, at the row's right end (phase 20 item 8). Test: `a_cut_table_draws_one_hidden_rows_mark`, a cut table in Short and in Long view, reads exactly one hidden-rows `…` from the buffer, at the row's right end.
2. **Met at phase 20's checkpoint** (phase 20 finding F030, nit; close it with this record). `measurement_cell` (`crates/cargo-tile/src/census/mod.rs` ~38) carries no `_for_test` suffix and is a `#[cfg(test)]` helper the tests share, and the file's tests stand in `mod tests` (~89). Nothing changes.
3. **The ellipsis is written once, and constants come in through imports** (phase 20 finding F031, nit, moved here open; close it with this item) (`crates/cargo-tile/src/constants.rs` ~196). Phase 23 removed `ARGUMENT_OMISSION_MARK` and the spaced argument mark (its item 2), so that part is met; what remains is the inline `crate::constants::` paths phase 20 added (about 23 lines across `render.rs`, `census/mod.rs`, `census/scan/census_tests.rs`, `census/process_identity.rs`, `app.rs`, `shim_registration/app_scenarios.rs` and `shim_registration/rows_readout.rs`) name constants at their use sites. Rule: the crate writes the ellipsis only in `ELISION`, and the spaced argument mark is built from it or drawn as a space then `ELISION`; a constant reaches its use site through a `use` import. Test: the render tests that read `…` and ` …` pass unchanged.
4. **A Long ancestry level cut between words keeps its mark against its text** (phase 20 finding F034, minor, moved here open; close it with this item) (shot `cdeeplong126x120`, the `verify.sh` argv). When the row budget ends at a word break, the row ends on the break's blank and the `…` follows it, so a blank cell stands between the last word and its mark. Rule (phase 20 item 4's, at a word break): the mark follows the last drawn character, with no blank cell between them. Test: `a_long_ancestry_cut_at_a_word_break_marks_its_last_word`, a Long ancestry level whose row budget ends at a blank between words, draws the `…` in the cell directly after the last word's final character.
5. **The frame that paints is the frame that decided** (`render.rs` `prepare_frame` ~395-425; `crates/tui_pane/src/tiles/draw/paint.rs:60` `draw_tile_grid`; `draw/measurements.rs:41` `prepare_grid`). The row-end reservation is decided from a preliminary drawing whose heights and `grid.progress()` read can differ from the painted frame. On a motion frame, or the frame a cell's allocation changes, a table can hide rows without its two mark cells (the hidden-rows `…` lands on the rightmost value's last cell), or hold the two cells with nothing hidden. Rule: the reservation is decided on the heights the frame paints, read from item 33's prepared plan, and a frame reads `grid.progress()` once. First check whether phase 20's round 4, which makes `draw_tile_grid` run one demands pass and one sync per frame, already closed it; if so, this item is only the test that pins it. Test: `every_moving_frame_reserves_the_row_end_only_when_it_hides_rows`, through the public draw path across a motion and an allocation change, reads every frame: a table that hides rows draws its mark in the two cells after its last value, and a table that hides none reserves no cells.
6. **Each cache keys on the inputs that change its answer** (`render.rs`: the `FrameTableColumnGrid` caches; `process_row` and its pid-indexed `by_pid: HashMap<u32, Vec<CachedProcessRow>>` ~4223). At phase 20's checkpoint process rows already look up by pid, and `pinned` is resolved before the layout cache is read, so a diagnosis of three scanned vectors, a quadratic lookup and a missing `pinned` key no longer holds. What remains: split the immutable frame context, fixed for the whole frame (the frame's column grids, view and the frame-wide `CommandView` Phase 23 added, whose hierarchy (`p`), argument detail (`h`) and default arguments are independent inputs), from the inputs that vary between lookups within a frame (row identity, `row_progress`, width, height); name the owner of each, and key every cache on the varying inputs it reads. Check each input against the current code first; one already keyed closes with its test. Test: `a_process_row_cache_answers_each_input_apart`, two rows on one grid that differ only in `row_progress` (and in any other varying input the check finds unkeyed), each draw their own form.
7. **One walk answers a table's header and remaining rows** (`render.rs` `table_needs_continuation` ~3124-3136, `select_table_slots` ~3263-3276). Each copies `process_table_plan`'s header and remaining walk. Rule: one walk, item 33's prepared plan, read by all three. Test: the existing render and table plan tests pass unchanged, and neither function holds a walk of its own.
8. **A profile label is whole or marked** (`render.rs` ~3317). At 18 cells a Long summary's command column had 8 cells, and `[release]` (9 cells) broke inside its label. The test that showed it no longer exists, and the current 18-column test (`a_narrow_long_summary_keeps_readings_after_the_command_leaves`, ~10299) deliberately drops `command`. Rule: a profile label is drawn whole, or cut with its `…` (Delegation Context invariant). Test: `a_narrow_long_summary_draws_a_profile_label_whole_or_marked`, a new fixture at a Long summary width whose command column draws a constrained `[release]` label, asserts the row's whole drawn form, so a label cut without its mark fails.
9. **A clipped-pane test reads what was drawn** (`crates/tui_pane/src/pane/frame.rs` ~790-795). `a_moving_pane_clipped_below_the_buffer_stops_at_its_last_row` asserts `buffer.area` equal to itself, so it passes whatever the pane draws. Rule: the test asserts drawn cells. Test: it reads back the drawn cells at the buffer's last row, so a clip that stops a row early or draws past the buffer fails.
10. **The changelog states what a user sees** (`crates/cargo-tile/CHANGELOG.md`). Phase 20's first new Changed line describes internals ("allocation probes", "solved layouts"). Rule: the line states the rule a user sees on screen and names no internal mechanism. Also (main read round 8): no line states the rule a user sees for a blocked nested cell, that a frame holding one wraps its commands between words to keep `cpu` and `mem` aligned across tables; round 8 removed round 7's clause and wrote no replacement (`CHANGELOG.md` ~62).
11. **`TileCells` stays dyn compatible** (`crates/tui_pane/src/tiles/draw/contents.rs:34`, `prepare_frame` at line 81). `prepare_frame` takes a generic `impl FnOnce`, so `TileCells` is no longer dyn compatible. Rule: `prepare_frame` takes `&dyn Fn() -> Vec<…>`, so the call stays lazy and the trait stays dyn compatible. `prepare_frame` was added by phase 20 on this branch and has not been released, so the Delegation Context's rule against changing an existing `tui_pane` public signature does not bind it; reword its Unreleased `tui_pane` CHANGELOG line to match. Test: `tile_cells_is_dyn_compatible`, a `tui_pane` test that binds a `&dyn TileCells<_>`, compiles.
12. **A Short command keeps the shared column end unless the reservation cuts its subcommand** (`render.rs` ~420-433). The whole-Short-command exception fires whenever the reserved width would wrap the whole Short name, which is broader than its rule: the exception applies only when the reservation would cut the command inside its subcommand. Rule: a cell tall enough to wrap `[release] cargo` / `build …` whole keeps the shared column end. Test: `a_short_command_wrapped_whole_keeps_the_shared_column_end`, such a cell's columns end with its neighbours'.
13. **Test counting stays out of product code** (`render.rs` ~494-495, ~5055-5058). Production `Cells::demands` reaches into `tests::FRAME_DEMAND_QUERIES` under `#[cfg(test)]`, so test instrumentation sits in the product's path, and the `thread_local!` stands above `mod tests`'s `use` lines. Rule: the test counts demand queries through a test provider, not product code; `Cells::demands` holds no `#[cfg(test)]` branch, and `mod tests` opens with its `use` lines. Test: the test that counts frame demand queries asserts the same count through the test provider.
14. **The row-end reservation is stable for an unchanged frame** (`render.rs` ~395-441; `crates/tui_pane/src/tiles/grid/tile_grid.rs:964` `preparation_allocations`; `draw/measurements.rs:41` `prepare_grid`). The reservation is judged unreserved against the layout the previous frame's demands produced, and that layout may have been sized from reserved demands. In a geometry where reserved demands rebalance the grid enough that the cutting table stops hiding rows, the decision can alternate between reserved and unreserved across frames, each flip a visible move. Rule: the decision is stable for an unchanged frame, because it is read from item 33's plan at the painted allocation rather than from the previous frame's demands. Test: `an_unchanged_frame_keeps_one_row_end_reservation`, such a geometry drawn repeatedly stays settled with one reservation.
15. **One account heading decision per table** (`render.rs` ~4133-4183, `TableAccountHeadings::of` and `heading`). `of` decides which accounts drop their tag; `heading` then falls back again on its own (`tail_heading(prefix)`, then `fitted_heading`), fallbacks that cannot run while both see one width but would draw an untagged path for a tagged account if they ever differed. Rule: `of` records the chosen heading form per group and `heading` only reads it, so one table never mixes a tagged and untagged heading of one account. Test: the mixed-account narrow summary test from phase 20 stays green, and `an_account_keeps_one_heading_form_per_table` draws a tagged and a path-only account at the width where one drops.
16. **The plan names the Short cut** (`render.rs` ~1694-1696, ~4425-4436). The argument mark's one ink moved to Phase 23 item 2: after Phase 23, ` …` marks only a width cut, so an uncut Short row draws no argument mark. What stays here: restate Phase 9's constraint for both views: a cut Long command has its column's full budget, keeps whole words and the separator before its mark, as the buffer test at `render.rs:7565` asserts (`cargo nextest run …`), and may leave unused cells after the mark; a cut Short command keeps whole words with ` …` directly after the last kept word, and unused cells stand after the mark, never before it. This item restates a constraint and has no test of its own.
17. **A hidden row's reading does not widen the drawn tables** (p20f, 2026-10-14). Hidden here means hidden by height; a row on `commands.hidden` leaves before any table at `Roster::visible` since Phase 21. A `blocked` reading on a row hidden by height widens the `state` slot of the tables that are drawn. Rule: the `state` slot's width comes from readings on drawn rows only (Phase 19's rule that the column stands only while a drawn row holds a reading). Test: `a_hidden_rows_reading_leaves_the_state_slot_width_alone`.
18. **The hidden-rows mark never doubles a row's own mark** (design check p20f: long40, clong40, clong64, cutfoot, cdrvlong64x20, c40d). A row cut by its own `…` or ending in an argument ` …` then draws the hidden-rows `…` one blank later (`-E test(gr… …`, `[ci] cargo nextest … …`), and the reserved cells cost long40 the `id)` of `test(grid)`, which fits whole without them. Rule: keep at least the column gap between the row's text and the hidden-rows mark (as w40_cut's `cargo tw …   …`), and a value that fits whole is not cut for the reservation. Test: `a_cut_row_keeps_a_column_gap_before_the_hidden_rows_mark`.
19. **A cell shows its reading somewhere** (design check p20f: c40d, c64, c64b, c48, c40, clong40, cdrvlong64x20, v23settled). A cell whose heading has no room for the gauge shows its reading nowhere, and a cut summary row can leave it on no part of the screen. Rule: drop the bar and phase word before the reading; where the path fills the heading row, draw the reading in the cell's row `state` cell. Test: `a_narrow_cell_heading_keeps_its_reading`.
20. **A table's empty `state` slot leaves when it would wrap a command** (design check p20f, c40d). One cell keeps an empty titled `state` slot and wraps `[ci] cargo nextest` / `run …` while another cell in the frame drops its empty slot to keep its command on one row. Rule: the empty slot leaves a table whenever keeping it would wrap or cut a command. Test: `an_empty_state_slot_leaves_before_a_command_wraps`. Round 8 widened it (main read round 8): a frame holding a blocked nested cell keeps every metadata column in every command table (`CommandNameFit::WrapToKeepResourceColumns`, `render.rs` ~3208-3214), so a 62-cell table leaves its command about 9 cells and `cargo nextest run …` wraps to three rows, while a neighbour with an empty `state` keeps its titled empty slot. The slot leaving moves only the columns after `state`, so it cures this without undoing F042's shared `cpu`/`mem` offsets. Also (p20k c64k): in a frame that hides rows, a cell drawing no hidden-rows mark still sets aside the mark's 2 cells, so its empty `state` slot leaves although the command fits beside it (`[release] cargo build …`, command at x30 against x38 in the other 7 cells; `cpu`/`mem` offsets stay equal). Also (p20x longcut, 90x30, after round 25): every cell table stops 2 cells short of its edge, so the Long cuts `--lib -…` and `--bin h…` end mid-token at x85 against the edge at x87, and the compiler columns end there too, while only the summary draws a hidden-rows mark. The same cell ran to the edge in p20w. Rule: a table that draws no hidden-rows mark keeps no mark cells, so its Long cut has the column's full budget; within it the cut keeps whole words with its mark directly after the last kept word (item 29), and unused cells may stand after the mark (phase 20 F068). Test: `a_long_cut_in_a_frame_that_hides_rows_keeps_no_mark_cells`: whole words, the mark adjacent to the last kept word, and no cells reserved for a mark the table does not draw.
21. **Other accounts keep their tags; headings that would read alike stay distinct** (design check p20f: w48, w48d, c48, c48x120, v23*, cn22d, cn16x120, cnd24x40). An untagged hana-linux-1 path among `[natepiano]` headings reads as the viewer's own account, and hana-linux-1 and hana-linux-2 headings both read `…/hana/hana`. Rule: at most one account per table goes untagged; the others keep their tags and cut to `…/<segment>`; two headings that would read alike are cut where their text still differs (`…linux-1/hana/hana`). Tests: `only_one_account_per_table_goes_untagged`, `two_runner_headings_stay_distinct`.
22. **A cut summary gives up every separator before hiding a process row** (design check p20f, w40_cut). One separator (p25 to p26) stays while the summary hides process rows. Test: `a_cut_summary_keeps_no_separator_while_it_hides_rows`. Also (p20j v23settled, 214x93): the summary gives up separator rows above two groups and keeps one above a third, while cells of the same grid column each keep a spare blank row above their foot.
23. **Long view wraps at the column's full width** (design check p20f, cnlong22x120). A 10-cell command column wraps at 8 cells, so 9-cell words split (`--profil`/`e ci`) and over-wide words break 2 cells short. Rule: wrap at the column's width less only the reserved mark cell, and only on the row that holds the mark. Test: `a_narrow_long_column_wraps_at_its_full_width`.
24. **Shot coverage** (design check p20f): the shot set adds a Short ancestry with a cut basename and a Long ancestry level cut by its budget, so phase 20's items 3 and 4 are judged on screen.
25. **Every external subcommand keeps its action word** (main read round 10, `render.rs` ~4687-4694, `constants.rs` ~277). A state frame's command budget keeps the action word only for `nextest`, so `cargo hack check …`, `cargo insta test …` and `cargo llvm-cov nextest …` budget as `cargo hack …`. Keep one following word for every external subcommand and none for a cargo built-in, then drop the renderer's nextest-only action-word rule. Keep `NEXTEST_SUBCOMMAND_NAME` for the parser's nextest owner (`census/command_text.rs`, Phase 32) and its valued-option tests. The executable's layout does not decide it: `cargo_split` (`command_text.rs` ~453–465) answers `ArgumentLayout::Cargo` whenever argv holds the cargo binary, `cargo hack check` included, and `ArgumentLayout::External` only for a `cargo-<name>` binary run directly. Rule: `CommandText` carries a semantic classification of its subcommand (for example `SubcommandKind::{BuiltIn, External}`), decided from the subcommand name against one table of cargo's built-in commands in `constants.rs`, the same for `cargo hack check` and `cargo-hack hack check`. The subcommand is read through the one option-skipping function over the options that `ValuedOptionScope::accepted_before_subcommand` (Phase 32d) selects from `COMMAND_VALUED_OPTIONS` (Phase 23's table; Phase 24 carries Phase 22 item 11), so `commands.hidden` still matches `cargo --config k=v port`. Tests: `an_external_subcommand_keeps_its_action_word_in_both_spellings` (`cargo hack check …` and `cargo-hack hack check …` budget as `cargo hack check …`), `a_built_in_subcommand_keeps_no_action_word` (`cargo check`, `cargo test` controls), and `cargo --offline hack check …` classifies as external.
26. **An excluded driver's reading falls back to its drawn child** (main read round 10, `census/scan/census.rs` `annotate_capture` ~1921 and `capture_boundaries.extend` ~1849, with `Census::prepare` ~314). `capture_boundaries` holds cargo pids that `select_rows` later drops (`commands.excluded`, rejected programs), so a drawn `cargo check` under an excluded driver with an unconfirmed capture borrows a reading that no drawn row owns. Test the endpoint against the drawn cargo set, or give the excluded driver's reading to its child.
27. **Round 10 tidy** (main read round 10). `NEXTEST_SUBCOMMAND_NAME` moves to its sorted place in the constants block, and the parser keeps it (item 25); the `FrameCommandColumns` doc moves its payload sentence to the `StateFrame` variant (`render.rs` ~3260); `annotate_capture`'s two disjuncts become one inclusive ancestor walk (`census/scan/census.rs` ~1967-1975); `longest_subcommand_width` is computed only for state-frame command grids (`render.rs` ~3192); `CHANGELOG.md` ~62 and ~64 merge into one line.
28. **A summary that hides rows keeps `mem` when it fits** (p20j and p20k w40_cut, 40x50, 26 projects). The summary draws `pid cpu state command` and drops `mem`, though `pid cpu mem state command` with `cargo tw …` and the hidden-rows mark needs 34 of its 36 cells. The row-end reservation takes width ahead of `mem`, the route F050 closed for command tables. Also seen in p20l c48 and cinst53-57.
29. **One Long cut form** (round 12 read). A Long command cut is word-cut (`--locked …`) in a frame that draws `state` and grapheme-cut (`--offlin…`) in other frames, so one row changes its cut form when another command blocks. Use the whole-word cut for every Long cut, and remove `CommandCutBoundary` with its redundant clear of the command cells; give `command_cut_boundary` its doc comment if it stays. Also (p20l): the summary and every frame without a state slot still cut Long arguments mid-word (clong64k summary `--no-…` for `--no-fail-fast`; clong64 `--off…`, `bu…`; clong40, cutfoot, cdrvlong64x20, cdeeplong126x120), and the clong64k cells put the argument mark at a fixed column with blanks after the last word. One whole-word form everywhere, mark directly after the last kept word.
30. **An empty column keeps the right title** (round 12 read). In a narrow frame that draws `state`, a table with no state value whose rows all lack a parent keeps an empty titled `state` column where round 10 drew an empty `parent` column: the reduced candidate restores no value, so the layout with the empty `state` slot stands. Drop the empty `state` slot there too.
31. **Round 13 tidy** (round 13 read, none a regression against main). (a) The whole-Short probe runs before `frame_row_end` is Reserved, so the probe and the paint judge a table with different budgets; set the row end before `preserve_whole_short_table` or keep it out of the budget. (b) `keep_state_before_parent` measures the frame's widest whole name, so a narrow blocked cell drops `parent` beside its own short name that fits. (c) A Kept Short argument word in a state frame can be cut mid-word (`cargo-nex…`); fold into item 29. (d) `mark_short_cut_line` turns a row whose first word fits but not with the mark into a lone ` …`; fold into item 29. (e) A state frame no longer drops an all-empty `parent` first (`NoParentReadings`). (f) A stateless table in a state frame never drops `parent`, so at the 40-cell floor with a ~20-cell subcommand word it drops `command`; let it drop `parent` before `command`. Cost: `select_table_slots` clones the grid on every call, Ordinary frames included; clone only in the StateFrame arm, and name the per-frame cost of the per-table budgets.
32. **Demand tests read the path the frame runs** (main read round 15, N3). `frame_tile_demands` (`render.rs` ~1384), `frame_group_height` (~1486) and `ancestry_source_demand` (~1538) are now `#[cfg(test)]` twins: production demands moved to `Cells::demands` → `PreparedCellRows::demand`, so 5 `tile_demands` calls (~14888, ~14957, ~15037, ~15195, ~15366) and 6 `group_height` calls (~9491, ~12646, ~12679, ~12989, ~12990, ~15715) check a path `draw_panes` no longer runs. The two paths agree today. Rule: those tests reach demands and group heights through `Cells`, as `content_row_use` already does (~1317-1324), and the three twins are deleted. Test: the 11 calls pass through `Cells` with their assertions unchanged, and no `#[cfg(test)]` copy of a demand function remains.
33. **One preparation contract: measurement and drawing read one table plan** (main read round 15, N4; it carries items 5, 7 and 14). Four pieces re-derive the plan the drawing uses: `TableHeightUse::at` re-derives `process_table_plan`; `PreparedSummaryRows::new` re-derives the slot choice (`for_table_slots`); `PreparedAncestryRows::at` re-derives `AncestrySource::plan` / `ancestry_fit`; and `prepare_frame` still measures with `frame_table_height` / `ancestry_source_row_use` (~428-447). They agree today, but an edit to one side alone would break "rows readout equals rows on screen" with no type or test to catch it. Phase 20 already runs one demands pass, one sync and one `grid.progress()` read per frame; what remains is that `preparation_allocations` hands endpoint geometry to preparation before the final drawing. Rule: one bounded preparation pass per frame builds each table's plan at most once per width and height (header and remaining walk, slot choice, row-end reservation, separator decision, ancestry fit), and measurement and drawing both read it; the duplicated derivations are deleted, not cross-checked. The plan's inputs split into immutable frame context (column grids, view, row set, retained widths) and the inputs that vary per table (width, height, pinned group, row progress), and it is keyed on the varying inputs alone. The reservation is decided on the allocation `draw_tile_grid` paints, until Phase 36's prepared frame carries that allocation. Tests: `prepared_rows_match_the_planned_table_at_every_height` compares the prepared `rows_drawn` and `rows_kept` with `planned_table_rows` at every height, over fixtures that hold `state`, compiler and managed columns, beside items 5, 7, 14 and 36's tests; no measurement function keeps a walk of its own.
34. **A narrow blocked cell keeps its state** (design check ux20z, c48k120; shared with main, routed here by the showrunner). The 48-column blocked cell draws `pid parent command` with no state column, so its row lacks `blocked` while the summary shows it. Rule: a blocked cell keeps its state column at 48 columns, so its row shows `blocked`. Test: `a_narrow_blocked_cell_shows_blocked`.
35. **The overlay's selected row is readable** (design check ux20z; shared with main, routed here by the showrunner). On the selected `view` row the label measures 2.02:1 and the value 4.12:1. Rule: the selected row meets 4.5:1. Diagnose first where its styles come from: the settings overlay's drawing (`crates/tui_pane/src/app_settings/draw.rs`), the selection styling in `crates/tui_pane/src/pane/state.rs`, or the palette (`crates/tui_pane/src/theme/`); a shared style reaches every app and theme, so check every shipped theme and name the changed owner in the summary. Test: `the_overlay_selected_row_meets_contrast` reads the drawn cell colors in each shipped theme.
36. **A table's group boundaries all have a separator or none do** (design check ux20aa, c48k120; showrunner's call, 2026-10-16). The summary keeps blank separators at 2 of its 13 group boundaries (before `/tmp/claude/lfo-gate31-fix` and `~/rust/cargo-liner-handler-enh`) and sets the other 11 flush, because Phase 19 gives separators up earliest first. Rule: within one table, every group boundary has a separator or none does; when the rows cannot hold every separator, the table draws none, and the freed rows go where Phase 19's spare-row rules send them. Process rows keep priority over separators (F007: a summary on loan gives up its separators before it hides a process row). `DirectorySeparation::YieldEarliest` implements the old rule twice, in `TableHeightUse::at` and in `process_table_plan`; the all-or-none decision moves into item 33's shared plan, so measurement and painting read one answer. Test: `a_summary_draws_every_separator_or_none`, a 13-group summary short two separator rows draws no blank row between groups; it steps every height between all separators and none, and at each the drawn rows and the rows readout agree.
37. **Long cells drop `start` and `dur` together** (design check ux20aa, clong64; showrunner's call, 2026-10-16; main does the same). Long cells draw `pid parent dur cpu mem command` after `start` leaves, while Short cells (c64) drop `start` and `dur` together. Rule: Long view drops `dur` with `start`, as Short does. Test: `long_cells_drop_start_and_dur_together` at clong64's width.
38. **Settings values stop short of the overlay's border and wrap between words** (design check ux21, s4_settings at 120x40 and 200x50; not caused by Phase 21, moved here under the hard-landing rule). A long settings value runs into the overlay's right border with no blank cell before it, and at 120x40 a wrapped file path breaks mid-word. Rule: every overlay value keeps one blank cell before the right border and wraps at a word or path-separator break, cutting with `…` only a single word longer than the row. Diagnose where the wrap is computed (`crates/tui_pane/src/app_settings/draw.rs` or the overlay frame) and name the owner in the summary. Test: `a_long_settings_value_wraps_short_of_the_border`, a list value longer than the row, reads a blank cell before the border on every row and no word split across rows.
39. **A column no row fills leaves the cell** (design check ux21, s5b_legacy2 at 120x40; not caused by Phase 21). A one-row `cargo handler` cell draws a titled `parent` column whose only value is blank. Rule: a table drops a column none of its rows fills, as item 30 does for `state`, unless the frame keeps it for alignment with a table that fills it. Test: `a_cell_without_parents_draws_no_parent_column`.

**Files:**
- `crates/cargo-tile/src/render.rs` — items 1, 4 to 8, 12 to 23, 25, 27 to 31, 32 (the demand tests through `Cells` and the three twins removed), 33 (one prepared plan for measurement and drawing), 34, 36, 37 and 39, item 3's imports, and their render tests.
- `crates/cargo-tile/src/render/frame_preparation.rs` — preparation inputs, retained tables, allocation decisions and row measurements.
- `crates/cargo-tile/src/render/command_cache.rs` — retained command widths, wrapped text, row cells and table layouts.
- `crates/cargo-tile/src/render/cell_paint.rs` — paint inputs, destination validation, command visibility and paint replay.
- `crates/cargo-tile/src/constants.rs` — item 3's ellipsis; items 25 and 27's constant, and item 25's table of cargo built-in commands.
- `crates/cargo-tile/src/census/mod.rs` — item 3's imports; `census/scan/census_tests.rs`, `census/process_identity.rs`, `app.rs`, `shim_registration/app_scenarios.rs`, `shim_registration/rows_readout.rs` — item 3's imports only. Items 26 and 27 also edit `census/scan/census.rs` (`annotate_capture`) and `census/command_text.rs` (item 25 carries the external-subcommand layout on `CommandText`).
- `crates/tui_pane/src/tiles/draw/contents.rs` — item 11; `draw/mod.rs` — its dyn-compatibility test; `draw/measurements.rs`, `draw/paint.rs` and `draw/paint_tests.rs` — item 5, only when phase 20's round 4 left the preliminary drawing.
- `crates/tui_pane/src/tiles/grid/tile_grid.rs` and `grid/allocation_tests.rs` — item 14, when its decision reads the grid's allocation there.
- `crates/tui_pane/src/pane/frame.rs` — item 9's test.
- `crates/tui_pane/src/app_settings/draw.rs`, `crates/tui_pane/src/pane/state.rs` — item 35's selected-row contrast, where its diagnosis places it, and any `tui_pane` palette file it names (named in the summary); item 38's settings value wrapping, in `app_settings/draw.rs` or the overlay frame its diagnosis names, and its test.
- `crates/cargo-tile/CHANGELOG.md` — item 10, and one Fixed line per item a user sees (1, 4, 5, 8, 12, 14); `crates/tui_pane/CHANGELOG.md` — a Changed line for item 11, and a Fixed line only when item 5 or 14 changes the tile preparation, painting or allocation modules.

**Workers:** 2 writers — the overlay and pane items (9, 35, 38) touch only `tui_pane`'s pane and settings-overlay files, disjoint from the cargo-tile table work and the shared tile drawing and grid files.
- `impl` — items 1 and 3 to 39 except 9, 35 and 38 (item 2 is met), and their tests: every cargo-tile file above, `crates/tui_pane/src/tiles/draw/{contents,mod,measurements,paint,paint_tests}.rs`, `crates/tui_pane/src/tiles/grid/{tile_grid,allocation_tests}.rs`, and both changelogs.
- `overlay` — items 9, 35 and 38: `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/pane/state.rs`, `crates/tui_pane/src/app_settings/draw.rs` and the palette files its diagnosis names, with their tests; posts its changelog text on the board for `impl` to write.

**Constraints from prior phases:** Phase 32b: `PreparedTables::prepare` keeps cell paintings across roster revisions, and `CellPaintSource::matches` validates each one against its sources. A change to table policy or to what a cell displays changes those sources, or the cell keeps a stale painting. Unchanged displayed values keep their reuse. Phase 32: `HiddenProcessCountRow::{Absent, Reserved}` and `ProcessRowVisibility` carry the `+N more` row, which has its own `…`. Arguments All reserve a count row for an overflowing cell and Default does not, so capacity differs between the two views. Items 1, 18 and 33 include the count reservation, the hidden totals and `ProcessRowVisibility` in the shared plan that Phase 36 consumes. Add All-view buffer tests for short commands, wrapped commands, multiple directories and height transitions; the ten fixtures that Phase 32 moved to Default do not prove this behavior. Phase 20's rules hold: each reading is drawn once, beside its own row; the hidden-rows mark stands at its row's right end in both views (item 8); a cut mark follows the last drawn character (items 3 and 4); a Short command overflowing its column has one form (item 5); the frame grid keeps shared offsets in every table, with phase 20 item 11's one exception. Phase 9: tables are planned before drawing. Phase 21: every render reader takes `&VisibleRoster` (`crates/cargo-tile/src/roster.rs` ~556), built once per frame in `render::draw`, so a group or nested subtree on `commands.hidden` never reaches a table, a column grid or a total; no item tests the hidden list again. Names that no longer exist at phase 20's checkpoint: `select_table_slots` (items 7, 31, 33; the slot choice is `for_table_slots`, `render.rs` ~4365), `CommandCutBoundary` (item 29), `keep_state_before_parent` (item 31b) and `CommandNameFit::WrapToKeepResourceColumns` (item 20); for each, check the item's rule against the current code first, and an item already met closes with its named test alone. Two visible-state tests from earlier phases bind here and pass unchanged: `an_unavailable_cpu_cell_draws_no_placeholder` (unavailable `cpu`, `mem` and compiler cells stay blank) and `default_ancestry_marks_only_a_cut_name_or_walk` (Phase 23's replacement for `each_short_ancestry_mark_reads_as_what_it_marks`). Phase 32 makes each ancestry row one row cut with `…` and keeps a table's group separators all or none: port items 4 and 24's ancestry-cut fixtures to that one-row rule; item 36's separator rule arrives as Phase 32 behaviour, and item 33 still owns its measurement through the one preparation contract. Phase 32 also keeps the pid and parent columns the same under every key toggle; the table-specific removal of empty columns in items 20, 30 and 39 is a different rule and stays. `draw_tile_grid` keeps its public signature (Delegation Context); Phase 36 rebuilds its internals as a read-only painter over a prepared frame, so item 5's test goes through the public draw path and carries over. No new `#[allow]` or `#[expect]`; no test takes a second. A `tui_pane` change also runs `verify.sh test` for `cargo-handler` and `cargo-port`. `TableHeightUse`, `PreparedAncestryRows` and `PreparedSummaryRows` live in `render/frame_preparation.rs`; table drawing remains in `render.rs`. Phase 43a already retains preparation across unchanged frames and keeps command widths and wrapping across resource updates, so limit cache changes to missing inputs and duplicate work that a test shows. Include each changed row, heading, column and mark policy in its preparation key. Preserve reuse when those inputs stay unchanged.

**Forward edits from Phase 46 (director, 2026-11-07):**
- `ExpandedCell` (`render.rs:599`) prepares full content, pins the painted column header, and keeps the header out of the shown range. Summary changes use the expanded width and keep every scrollable row. Extend the header and End tests to the rows this phase adds or changes, with an exact shown range.

- Item 11's dyn-compatible `TileCells` change covers `prepare_moving_frame` (`tui_pane/src/tiles/draw/contents.rs:86`) as well as `prepare_frame`, and the caller `ExpandedCell::prepare_frame` (`render.rs:599`). The `&dyn TileCells<_>` test needs both.

**Forward edits from Phase 49 (director, 2026-11-07):**
- Phase 49 footer contract: the summary foot is `SummaryFoot::WithOptionalSuffix` (`tui_pane/src/tiles/draw/foot.rs:17`), prefix `mem`, suffix `disk`. The unit is `G`. Memory has one decimal. Disk has one decimal below 10G and is whole from 10G. At a narrow width the foot keeps `mem` and drops the disk suffix and its gap. `FreeDiskSpace::Unavailable` (`census/scan/refresh.rs:143`) omits the disk value and its gap, with no user remedy.
- `SummaryFootPreparation` (`render/frame_preparation.rs`) retains the foot by memory, rounded disk text and theme. `PreparedPaneFrame` joins it to the independently retained tables. A table change keeps this boundary. `disk_text_changes_reuse_table_preparation_and_update_only_the_foot` and `an_expanded_summary_updates_its_disk_footer_after_scroll_and_resize` pass unchanged.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile` and `... lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; each item's named test passes; no test takes a second or more.
- Item 35: a rendered contrast check of the overlay's selected row in every shipped theme reads 4.5:1 or more.
- Unit director's captures of the shot set Phase 20 used: every hidden-rows `…` stands at its row's right end, on settled and moving frames, and no value's last cell carries it; no `…` stands apart from the text it cuts; no profile label is cut without a mark.
- A fresh helper's design check of those shots: pass.

### Phase 36 — Every cell moves toward one target layout  · status: todo

**Depends on:** Phase 20, Phase 35

#### Work Order

**Goal:** The tile grid's motion is rebuilt on one model (the user, via the showrunner, 2026-10-12: "Do the rewrite"). One pure target layout is computed fresh on every change (a scan, `+` or `-`, a resize). Each cell keeps the one rect it shows now and moves toward its target by a deterministic `step(displayed, target, dt)`. Any change retargets every cell from where it stands, at once. There is no step queue: `pending`, `Transition.source`, `motion_heights`, `retargeted_destination`, `DestinationDepthStatus` and the in-motion special cases all go. Drawing reads state and never writes layout back, and a cell is always one rect, so its rows never exist in two places. A cell's group title reads as a named title. Every motion case below, every motion ruling in force, and every kept test of visible behaviour passes on the new model, and the live capture counts 0 failures on each rule below.

**Spec:** Evidence for every item: the round 3 build's live capture B, `results_p14.txt` sections `P14d.1` and `P14d.4` (copied into the delegate session directory before dispatch). Notation: start-end(h height, N content rows / R rows drawn, CUT = content hidden, IN = entering this frame); times are from the key.

1. **A cell that opens while another slides in starts at its opening height.** Known at Phase 14's checkpoint and worse than the Phase 13 build; Phase 14's last repair turn stopped without a repair. First `+` of a round, both rounds, at +0.009 s and +0.014 s, with a command's cell 0.10 s into its entry slide:
   `+0.000s 0-20(h20,N9/R19) 20-41(h21,N10/R20) 41-48(h7,N10/R6 CUT)`
   `+0.009s 0-12(h12,N9/R11) 12-31(h19,N10/R18) 31-43(h12,N10/R11 IN) 43-48(h5,N10/R4 CUT)`
   A cell appears at its end height in one frame, the summary drops 8 rows in that frame, and the cell that was sliding in falls from h7 to h5. On the Phase 13 build the same key brought one piece that entered at h2 and slid for 0.62 s. Required: the opening piece slides from its opening height; a piece still entering stays an entering piece and its height does not fall while its band has room (item 16); the residents move along the clock.
2. **A cell admitted into a running motion slides from its opening height.** Known at Phase 14's checkpoint and worse than the Phase 13 build, as item 1. Third `+` of a round, both rounds, at +0.205 s and +0.191 s:
   `+0.189s 0-17(h17,N15/R16) 17-22(h5 CUT) 22-30(h8 CUT) 30-39(h9 CUT) 39-48(h9 CUT)`
   `+0.205s 0-17(h17,N15/R16) 17-22(h5 CUT) 22-28(h6 CUT) 28-35(h7 CUT) 35-42(h7 CUT) 42-48(h6,N10/R5 CUT)`
   A command's cell enters at h6 in one frame and three cutting mates lose 2 rows each. On the Phase 13 build it entered at h2 and reached h5 0.71 s later.
3. **An empty cell slides out across a summary depth step.** 1 and 3 cells per round still lose their last 3 or 4 rows in one frame, each on a frame where the summary's depth steps (Phase 13 build: 4 and 3). Worst, second round, tenth key (`-`), +1.294 s, from h4:
   `+1.284s 0-11(h11,N12/R10 CUT) 11-16(h5,N0/R4) 16-20(h4,N0/R3) 20-29(h9 CUT) 29-38(h9 CUT) 38-48(h10 CUT IN)`
   `+1.294s 0-14(h14,N12/R13) 14-19(h5,N0/R4) 19-29(h10 CUT) 29-38(h9 CUT) 38-48(h10 CUT IN)`
   Required: a leaving piece is drawn until its slide ends, also across a depth step.
4. **The summary moves along the clock.** The summary changes height by 4 rows in one frame on 4 keys of the two rounds while its content does not change (the Phase 13 build does the same on 3). Required: no resident changes height by more than 3 rows in one frame, outside the named transfers of items 5 and 15.
5. **A rule for a cell that is itself sliding.** Every frame the resident spare-row count still binds (25 and 95 per round; Phase 13 build 267 and 186) is one of two forms: the cutting mate entered 0.10 s before the key and is still sliding in; or the holder's own content shrank less than 0.75 s earlier and it now slides down where it dropped in one frame before. The second form sets item 4's rule against the spare-row rule. Ruling (showrunner, 2026-10-12): a cell whose own content shrinks is a resident, so it gives its rows at once to a mate that cuts, with no slide; rows no mate wants follow the existing clock. Item 4's limit therefore excepts a resident giving rows to a cutting mate, and the first form (a mate still sliding in) is an entering piece under Phase 14's ruling. The capture tool's classes follow this.
6. **One motion per key, read on keys free of arrivals.** The reversal and one-motion gates could not be judged: no key of the workload was free of a command arriving in its window, and arrivals aimed at the second and third keys land 0.19 to 0.22 s after them (0.02 to 0.05 s before them on the round 2 build; cause not determined). Find the cause of the shift, give the workload keys with no arrival, then require 0 reversals and one motion per key on those.
7. **The checks Phase 14 did not run:** the changing-width capture for band escapes, and the frame probe (`cap6`) repeated on a quiet machine.
8. **A staying cell keeps its rows on a key that frees room.** Phase 14's design check on its checkpoint build, `motion_09_minus` line 22 and `motion_10_minus` lines 26 and 31: a cell that stays shows two process rows before and after the key (`motion_08_minus`, `motion_11_minus`) and one row with `…` 0.33 s into it, while the summary already stands at its full 16 rows with four blank rows between projects. A piece is leaving in both shots, but the pieces cut are staying ones. One frame per key cannot tell a dip inside the key from a settled six-cell layout, so read it on the live capture first. Required: on `-`, a staying cell keeps the rows it had, and the summary grows only by rows the leaving cell has given up.

Diagnosis from Phase 14's last repair turn, for items 1 and 2: `open_waiting_commands_in_current_motion` changes the slots while it keeps the running transition's source and clock, and seeds the new piece's previous height to 1; `drawing_at` then draws the new piece at that shared clock's position. A piece admitted into a running motion has no admission time of its own. Two fixtures built from the live heights did not reproduce the entry-at-height frame, so the first step is to instrument the admission path on the live workload and record the transition's source, destination and clock at the frame of each case. The design below answers it: each admitted cell owns its `AdmissionTiming`, and the joint step moves the dividers under item 16's rule.

Each repaired item has a render-path regression that fails without it, built from the live roles (resident, entering and how far into its slide, leaving; opening queued or cell claimed), and its fixture joins `every_frame_of_a_key_keeps_the_motion_rules`.

9. **A summary that can give a row gives it on the arrival frame, even while a piece is leaving** (the showrunner's placement, 2026-10-12, from phase 15's live frame check, which read worse than phase 14 on this alone). Seen: a third project arrives while the summary stands eight rows high with no spare row and a leaving piece two rows high stands below it; for two frames (36 ms) the summary draws its header, two projects, one blank row and its foot, with the third project hidden behind the cut, and only then gives the row. Rule, written as behaviour so it holds for whatever motion model this phase lands: on every frame, a summary that hides a project behind its cut draws no blank row above its foot; when an arrival leaves one of its rows unused, its height drops on that same frame, whatever else in its column is moving. Cause in today's code, for reference only: `snapped_band_piece_dividers` in `crates/tui_pane/src/tiles/grid/bands.rs:976` (`piece_span_bounds` at 1158, `snapped_piece_fit` at 1229), on `FilledBandPath::NoFilledPath`, returns the heights held before whenever their dividers reach the band floor, without rechecking that a resident now leaves a gap; a leaving piece two rows high makes the joint search fail (`piece_span_bounds`'s leaving guard and `snapped_piece_fit`'s minimum-height rejection). That file is the same in phases 14 and 15, so phase 15 did not cause it. Evidence: `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/live15_reply.txt`, `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/probe12/out/results_p15.txt` (sections `P15 gates` and `P15 control`), and the kept captures `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/probe12/keepcap_c_p15b/c_cells7_r1.json` (key8 `-`, +0.317 s) against `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/probe12/keepcap_c_p14ctl/c_cells7_r2.json` (key8 `-`, +0.223 s). Test: `a_summary_gives_its_unused_row_on_the_arrival_frame_while_a_piece_leaves`, through the real draw path, reading the drawn buffer back on every frame.

10. **A job cell that changes columns lays its content out once** (add-on, answered by the user via the showrunner, 2026-10-12: covered by the rewrite). A job cell's heading and ancestry are its name, and today its two pieces lay content out separately (`draw_placements`, `TileCells::draw`; cargo-tile's `Cells`), so a row shows twice or drops out while the cell changes columns, a closing column included. Evidence: `P/ph3s/shots/ph3s_on_12_close_motion.png`, `ph3s_off_09_close_later.png`, capture `P/ph3s/off/cap.json` key 13 at 1.2 s. On the new model a cell is one rect, so this holds by construction; a render test with a job cell crossing columns in both directions shows each content row at most once per frame.
11. **A `-` pressed during a motion starts at once** (add-on, answered as item 10). Today a queued key waits in `pending` until the preceding transition's 720 ms timer ends, 0.64 to 0.93 s late with nothing changing on screen (`P/ph2d/off/ph2d_off_08_close_motion.png`). On the new model a key retargets every cell on the frame it arrives; a test presses `-` mid-motion and sees every cell's target change, and the first moved frame, within one frame.
12. **A key that lands while a command's motion runs joins that motion** (from phase 14's live check, by the showrunner's landing call, 2026-10-12). Evidence, the controlled workload (200x50, cells view) on phase 14's build after its second repair round: a `+` that lands 7 to 112 ms after a command claims a cell waits 0.96 to 1.08 s for its slot, because the key queues behind the command's motion. In that wait the summary takes 4 rows and gives them back when the slot enters (second round, sixth `+`: height 13 to 17 at +0.451 s, 17 to 13 at +1.077 s, 13 to 11 at +1.438 s), and a command cell takes one row and gives it back (third `+`, both rounds: 5 to 6 at +0.281 s, 6 to 5 at +1.421 s). Rule (showrunner's call, 2026-10-12): a key that lands while a command's motion runs joins that motion, and no piece takes a row it gives back. This holds for a command whose cell opening has not yet entered, one that has claimed an existing cell, and one whose cell has entered; on the new model the key retargets every cell on its frame. Tests, on the render path: `a_key_landing_after_a_command_claims_a_cell_joins_its_motion`, `no_piece_takes_a_row_it_gives_back_when_a_queued_key_enters` (no piece's height rises and then falls back within the key). Gate: the unit director's capture reads 0 reversal flags and one motion on every key that lands within 0.15 s after a command arrives or claims a cell.
13. **A cell's group title is a named title** (the title is drawn by the painter this phase replaces). `TileCells::group_title(&Id) -> Option<Line<'static>>` (`crates/tui_pane/src/tiles/draw/contents.rs:142`) becomes `TileGroupTitle::{Untitled, Titled(Line<'static>)}`, with its match in the drawing, its exports from `tiles/mod.rs` and `lib.rs`, and every implementor: cargo-handler (`crates/cargo-handler/src/render/mod.rs:982`, `group_title`) and the title providers in `draw/tests/mod.rs` (lines 126, 291 and 1096). cargo-tile keeps the provided default, which answers `Untitled`. What the screen shows does not change. cargo-handler's title is a line of several spans with their own styles: an agent title is the session name in `Role::Session(hue)` style followed by the role suffix in the hierarchy role tint, and an unassigned title uses `title_color()`. The regression stays: `tile_title_suffixes_use_showrunner_and_unit_role_colors` (`crates/cargo-handler/src/render/role_title_tests.rs`) reads the title through `group_title` and checks the role suffix ink in the drawn buffer. Its port takes the line from `TileGroupTitle::Titled`, keeps that assertion, and adds one for the session name's `Role::Session(hue)` ink, so it fails if the title loses either style.
14. **A cell that changes column or order position closes out of its old spot, then opens in its new one** (showrunner's ruling, 2026-10-12). Over the first half of its travel the cell closes out of its old spot; over the second half it opens in its new spot. It is one rect at every frame, never overlapping or crossing another cell, and it keeps its content, title and focus throughout. Commands that end together close together, not one cell at a time. A straight slide drawn over its neighbours is not allowed. `a_cell_changing_column_is_drawn_in_both`, `commands_ending_together_close_one_cell_at_a_time` and the reorder tests (`a_cell_the_order_overtakes_closes_and_comes_back_in_behind`, `the_order_changing_cascades_down_the_rest_of_the_grid`, `no_step_of_a_reorder_draws_two_cells_crossing`, `a_reorder_leaves_the_cell_count_where_it_found_it`, `focus_waits_for_a_cell_that_closed_to_let_the_order_through`) are ported to this rule, each keeping its titles, content identity, focus, final order and non-crossing assertions.
15. **Filled heights win over the three-row step** (showrunner's ruling, 2026-10-12). When the nearest height a resident's content fills is more than three rows from its displayed height, the cell moves straight to that height as a named transfer, as a content shrink does under item 5, so no blank row ever shows. Item 4's limit excepts this transfer. `a_height_search_reaches_the_nearest_valid_height_each_way` keeps its 19 and 27 heights.
16. **An entering cell keeps its displayed height while its band has room** (showrunner's ruling, 2026-10-12). A retarget that asks an entering cell for less height than it displays leaves the cell at its displayed height while its band can hold it. When the band cannot fit it, the cell moves toward its new target at the resident rate, and its admission clock continues; it is never restarted.

17. **Two motion findings from phase 19's re-shoot** (the showrunner's routing, 2026-10-13; shots `ph19b`, captures `cap7c` and `cap6`). First, each key's last moving frame still differs from the settled frame after it: cap7c 7/14 and 10/14 keys, cap6 6/14 and 5/14; the phase 18 build reads 10/14 and 10/14, and 4/14 and 4/14, so it predates phase 19. The pattern: resident cells one row taller at the last moving frame than settled, cells below one row up, the entering cell one row taller settled. Second, one column widened by one cell during motion: in cap6 `w200_r1`, the cell's `parent` column widened at 4.678 s and `start`, `dur`, `cpu`, `mem` and `command` each moved one cell right. Today `FrameColumnGrids::for_frame` (`crates/cargo-tile/src/render.rs` ~3111, taking `&VisibleRoster` since Phase 21) rebuilds the parent widths on every frame. Rule: the last moving frame equals the settled frame (this phase's one-target-layout design), and the column widths a motion draws are one named policy, `MotionColumnWidths`, held for the motion's lifetime: computed at the motion's start, retargeted only at a key or scan retarget boundary, widened (never narrowed) while the motion runs when a longer reading or pid arrives, and narrowed only on the settled frame. It keeps phase 19's session-retained `cpu`, `mem` and compiler widths, and it reads the motion's start, retarget and settled states from the prepared frame. Tests: a motion whose final frame is compared cell for cell with the settled frame after it; `a_longer_parent_pid_widens_its_column_once_during_a_motion`, a cell whose rows gain a longer `parent` pid during a motion, draws every column at one offset in every frame before the pid arrives and at one offset in every frame after it; `motion_column_widths_narrow_only_on_the_settled_frame`, a motion whose rows lose their longest pid, keeps every offset until the settled frame.

18. **Columns fit the rows the final allocation draws** (phase 19's routing, 2026-10-13). `possible_frame_rows` (`crates/cargo-tile/src/render.rs` ~3189) filters rows only by the frame-height ceiling, so a long row the final allocation leaves out can still remove `cpu` and `mem` from every table. Rule: preparation fits the frame's columns to the rows the final allocation draws, with a bounded fit and the shared measurement (`Cells::row_use`, `FrameColumnGrids`), so no row the screen does not show takes a column from one it does. Test: `an_allocated_out_long_row_keeps_cpu_and_mem`, a render test where the final allocation leaves a long row out, draws `cpu` and `mem` in every table.

19. **An idle tile draws no frame** (the showrunner's placement, 2026-10-14, because this phase replaces the stepping clock). cargo-tile repaints 25–49 frames a second while nothing on screen changes (phase 20's shots `p20d` and `p20e`, 2026-10-14): each such frame is 25 bytes, an attribute reset and a cursor hide, with no cell written and no ink change. Rule: with no change in content, motion, fade or clock text, no frame is drawn. The status line's once-a-second clock frame (Phase 33: uptime and CPU) is the one frame an idle tile draws; the idle-count test injects a clock and counts frames other than that tick. Gate: a test that counts drawn frames over an idle stretch and asserts zero beyond the scheduled once-a-second status frames, plus a `CARGO_TILE_FRAME_LOG` reading of an idle live tile showing no frames beyond those status frames.
20. **A closing cell moves the summary's foot one way** (p20f shot clong40, 2026-10-14). When a cell closes, the summary's foot row moved down five rows and back within 0.6 s. Rule: under the target-layout motion, a closing cell moves each neighbour's edge toward its target only, never past it and back. Test: `a_closing_cell_moves_the_summary_foot_toward_its_target_only` steps the motion and reads the foot row's y as monotone.
21. **Rows paint as a growing tile reaches them** (the showrunner's ruling, 2026-10-14, from the user's screenshot of tiles mid-expansion drawing `content rows: 0` and no text). While a tile grows, each content row paints, fading in quickly, on the first frame the tile's current height covers its rect, not when the tile reaches full size. A tile that appears and leaves within its animation still shows its text while open. Built on the read-only painter.

22. **Column-crossing motion keeps row identity (F020 fallback)** (deferred by the showrunner 2026-10-18; next-items "Column-crossing motion preserves row identity"). Evidence: `/tmp/claude/delegate/03f9b80e-dbe5-47c6-b29e-a7e6587ab721/phases/22/review_findings_5.txt` item 2. On the Phase 22 round tree, a fragment that failed the required-block check was drawn as `PaneFrame::new(piece.placement.frame.clip())`, which sets its shift to zero and lays its content out again from the clip's first row: a nine-row source at y=8, shifted −9 into a four-row clip at y=4, restarts its content instead of going on with its slide. That tree's test `a_one_row_provider_never_draws_a_crossing_readout_alone` drew the same text on every row, so it could not see a restart. Neither that fallback nor that test is on main; Phase 28 did not land, so this phase adds the regression. Rule: content that crosses a column keeps its sliding row identity on every frame, or it is not drawn; it is never laid out again at its clip. On the one-rect model this holds by construction; this phase proves it. Test, red first on a re-anchoring fallback: `column_crossing_content_keeps_its_row_identity`, through `draw_tile_grid` with a provider that draws a distinct text on each row, with injected time; on every frame each drawn row's text is the source row its shift places there, or the cell is not drawn.

**Design** (approved by the showrunner, 2026-10-12). The rewrite keeps one target layout and one displayed rect per cell. Neighbouring cells share separators, conserve their column's rows and hand useful content rows over in one frame, so the grid advances by one joint step rather than by independent rect interpolation.

*Model.* The public `TileGrid`, `TileDemand`, `TileDemands`, `TileContent` and `TilePlacement` interfaces stay. These semantic types replace the internal motion implementation:
- `CellIdentity<Id>::{Summary, Command(Id), Vacant(VacantCellId)}` names a physical cell apart from its displayed number and content. A command claiming a vacant cell keeps that cell's identity, rect and admission timing.
- `DisplayedCell<Id>` owns its identity, current content, one displayed rect, presence and travel clock. It owns no source rect, endpoint copy, previous-height vector or queued destination.
- `CellPresence::{Resident, Entering(AdmissionTiming), Leaving(DepartureTiming)}` selects the motion rule. An existing entry stays entering across a retarget; claiming its cell does not create another entry.
- `CellMotionState::{Settled, Travelling(TravelTiming)}` (internal) says whether geometry needs another frame. The public `CellTravel` in `tui_pane/src/tiles/grid/pieces.rs:46` keeps its meaning and its method. `TravelTiming` holds elapsed and remaining duration, not a source layout.
- `TargetLayout<Id>` holds the settled cell rects, column bands, summary depth and span, focus navigation lanes and the resolved display mode.
- `CellTarget::{Present(Rect), Withdrawn(ExitEdge)}` tells an ordinary destination from removal. A withdrawn cell keeps its displayed rect until its slide has finished.
- `CellRowUse` holds painted rows, retained rows and `ContentFit::{Filled, Whole, HidesContent}`. Painted rows decide whether a receiving allocation draws more; retained rows protect donors.
- `RowTransfer::{None, ImmediateContentRelease { donors, recipients }}` records the one event in which every participating cell may exceed the three-row limit (items 5 and 15). The update produces it; the painter never infers it.
- `RetainedCellAllocation` keeps today's stepped-demand stickiness as an explicit input to layout, never a second geometry authority.
- `GridCellHit::{Outside, Summary, Cell(..)}` and `RemovalCandidate::{NoneAvailable, VacantCell(..)}` replace the internal optional answers; the public `cell_at -> Option<usize>` converts at its boundary alone.

*Modules.* Target allocation lives in `tiles/target_layout.rs`, motion in `tiles/motion.rs`, identity and placement types in `tiles/grid/pieces.rs` and `tiles/grid/visual.rs`, and the existing public methods in `tiles/grid/tile_grid.rs`. Timing literals stay in `tiles/constants.rs`; the ripple settings leave `tiles/settings.rs`. `tiles/mod.rs` and `lib.rs` own the additive preparation API.

*Target layout.* `target_layout(inputs, content) -> TargetLayout` is pure in its observable result. Its inputs are the area, growth, floor and view settings, the reconciled cell identities and order, focus, the current content revision, the retained demand and allocation policy, and immutable content measurements. It reuses the existing `columns`, `fits`, summary depth and span rules, apportioning, summary priority and focus priority. It evaluates content at the candidate destination widths before it publishes a target, so no provisional depth is published and repaired on the first draw; the width and depth search is bounded and deterministic, with an explicit tie order. A fresh target is computed whenever a scan changes relevant content, a key changes cells or focus allocation, settings change, or the terminal resizes. An unchanged observation may reuse its result, but never restarts travel because another frame occurred.

*Step.* `step(displayed, target, dt) -> DisplayedLayout` takes the whole displayed grid. It first proposes each cell's advance, then resolves shared column boundaries and useful row transfers jointly. It uses integer fixed-point coordinates and an injected duration, and reads no `Instant` inside layout or stepping. For unchanged topology and unrestricted geometry, the residual easing fraction `(ease(elapsed + dt) - ease(elapsed)) / (1 - ease(elapsed))` advances the current rect toward its target without a stored source rect; it clamps at the remaining duration and lands exactly on the target. Stable cell order decides rounding and every allocation tie.

*Joint projection.* The step conserves each band's rows, shares each separator, respects resident floors, prevents ordinary resident height reversals toward a fixed target, and selects useful filled allocations from the current content measurements. A small leaving cell does not invalidate the resident allocation: its timed allocation is accounted for apart from resident content-fit eligibility, so today's `FilledBandPath::NoFilledPath` fallback that keeps an obsolete gap goes.

*Retarget.* A retarget replaces the whole target at once and starts from the stored displayed rects. It never rebuilds positions from a previous endpoint or advances an old queued step. Entering and leaving clocks survive content-only retargets; a newly admitted cell gets its own clock. A new key updates every target on its frame, with resident travel timing assigned to that key; unchanged frame observations do not extend its deadline.

*Opening and withdrawal.* A new opening starts at its first drawable allocation (border, foot and one content row; Phase 22 item 6), never border-only, and its `AdmissionTiming` takes duration and easing from Phase 22 item 9's one lifecycle timing: 720 ms from its own admission, whatever another cell's elapsed motion, with its text fading in over the same span. A leaving cell's `DepartureTiming` takes the configured fade from the same authority, so its text alpha and its geometry read one timing; it releases rows along its slide, owns its last drawable rows until its terminal frame, and is erased on the frame it can no longer draw a content row. A group hidden mid-run is no departure (Phase 22 item 6): it leaves the target layout at once, with no `DepartureTiming`, and its neighbours retarget from where they stand. A `CellIdentity::Vacant` cell opened by `+` is an intentional empty slot and keeps its own opening. Item 16 sets what an entering cell does when a retarget asks it for less than it displays.

*Preparation and painting.* A preparation operation observes `TileCells`, resolves the target, gathers the existing shared row measurements, steps once, and publishes an immutable `PreparedTileFrame`. The painter reads that frame and calls the content provider once per visible cell. `draw_tile_grid` keeps its public signature and performs no layout mutation; cargo-tile and cargo-handler prepare before calling it. Hit-testing and navigation read the same published geometry. Phase 20's allocation solver (F054, `DonorSupply`/`RecipientContentGain`/`ColumnTransfer` with a lexicographic termination measure, and F007's summary loan, whose separators yield all together under Phase 35 item 36, process rows first) is today's useful-row allocation: the joint projection keeps both guarantees and their `grid/allocation_tests.rs` regression tests, and Phase 22's saturation cap (a cell no taller than the allocation past which more height draws no row, borders and foot included, the floor taking precedence). The prepared frame also carries the clipped summary frames the painter draws (today `TileGrid::painted_summary_frames`, recorded during painting), so toast placement (`Toasts::area_avoiding_frames`, read by cargo-tile and `draw/paint_tests.rs`) reads them from the prepared frame and the painter records nothing. `TileView` is `Summary` and `Cells` only (Auto removed in phase 20). cargo-tile keeps its immutable `FrameColumnGrids` and the shared `Cells::row_use` measurement in `render.rs`: measuring at candidate sizes is distinct from drawing, and each displayed cell gets one final content layout and one paint. Phase 16's `CellReadoutFade`, focused ground selection, summary-foot fading and whole-or-absent readout candidates stay.

*Removed mechanisms and their replacements:*

| Removed | Replacement |
|---|---|
| `pending`, `Step`, the queue and advance machinery | Reconcile identities and publish one latest target |
| `Transition.source` | The rect in each `DisplayedCell` |
| `motion_heights` | Heights already in those rects |
| `retargeted_destination` | The sole `TargetLayout`, recomputed from current inputs |
| `DestinationDepthStatus` | Complete width and depth resolution before the target is published |
| Admission into a shared running clock | Per-cell `AdmissionTiming` |
| `HeightSnap::previous_height -> Option<u16>` | No height-history lookup; current geometry is always present |
| Draw-time selected-height and layout writeback | The update's joint step and the immutable prepared frame |
| Two `TilePiece`s and a `PieceName` per crossing cell | One displayed cell and one content paint |

*How each item and ruling holds:*
- Entering and leaving slides: presence controls timed opening and withdrawal, apart from residents' content allocation.
- Resident content shrink (item 5): usable rows go to cutting resident mates on that observation's frame; donor and recipients form one `RowTransfer`. Rows with no useful resident recipient follow the clock.
- Spare rows: useful transfer among residents holds on every prepared frame and among all cells of a settled layout; entering and leaving allocations keep their slide exemption.
- Item 1: a second opening starts at its own opening allocation, while an existing entry keeps its identity and admission clock.
- Item 2: admitting a command during motion creates one new timed entry, or claims an existing physical cell.
- Item 3: a leaving cell's visibility depends on its withdrawal lifetime, not on a summary depth transition.
- Item 4: the joint step applies the three-row resident limit, except the named transfers of items 5 and 15.
- Item 6: target replacement leaves no second queued motion; reversal is judged between actual external changes, on workload keys free of arrivals.
- Item 7: the changing-width containment captures and the quiet-machine frame probe stay.
- Item 8: on removal, staying cells keep the rows they already paint; summary growth takes only rows the withdrawal released.
- Item 9: resident row use is recomputed on the arrival frame and the summary's unused row is released before stepping; a two-row leaving cell cannot veto that transfer.
- Items 10 and 14: one displayed rect gives one content layout and one paint per frame.
- Items 11 and 12: a key publishes every affected target on receipt and moves toward them within one frame.
- Item 13: the painter matches `TileGroupTitle`.
- Item 17: the step lands exactly on the target, so the last moving frame is the settled frame; `MotionColumnWidths` holds column widths for the motion's lifetime.
- Item 18: preparation fits columns to the rows the final allocation draws, through the shared measurement.
- The readable floor: cargo-tile's six-row and 40-column floors hold for resident cells and settled layouts; cargo-handler's framework floors stay.
- Phase 14's live guarantees: the seventh-key empty-cell floor, no summary blank row hiding content, one physical entry per `+`, and containment hold as explicit buffer or capture assertions.
- Probe cost: the shared measurement cache stays; a seven-cell moving frame makes at most 29 row-measurement calls, and painting causes no repeated measurement.
- Height and admission types: the current rect and named admission timing replace optional history and the height-1 sentinel.

*Cost.* Today a moving frame resolves endpoint layouts, builds possibly duplicated pieces, gathers row probes, searches band dividers, stores selected heights and rebuilds drawing geometry; `nearest_band_piece_dividers` also clones growing divider vectors while it explores candidates. The new frame makes one measurement and preparation pass, one joint step and one paint per cell. Rect proposals and projection are linear in cells and bands; the useful-height allocation still needs a bounded joint search for sparse content steps, so it is not claimed linear. It stores scalar scores and predecessor indices instead of candidate divider vectors, indexes row answers by cell, and builds the chosen rects once. Painting triggers no allocation search and duplicates no content plan.

**Test map.** This map covers the 163 former `grid.rs` tests and 78 former `draw.rs` tests at phase 20's checkpoint. Phase 29 moved them into `grid/` and `draw/` test files; today those files hold 178 grid tests and 86 draw tests, and the added tests are listed after the map. *Keep* means the test body and its assertion stay unchanged, with any pure helper they need. *Port* means every visible assertion stays while fixtures that write `held`, `Transition`, source geometry, queued states or height vectors become displayed-cell and target fixtures. Arbitrary `drawing_at(raw)` samples become deterministic update-and-step samples (independent fixture states where the samples are deliberately unordered); queue completion becomes stepping to settled geometry; and a test inspects the immutable prepared frame instead of rebuilding an endpoint during drawing.

Former `grid.rs` — keep (now in `grid/allocation_tests.rs`, `grid/tile_grid_tests.rs`, `grid/arrangement_tests.rs` and `grid/tests/motion.rs`):

```text
the_configured_tile_floor_fits_each_column_count_at_its_shared_width
the_framework_floor_still_fits_three_columns_in_twenty_two
fits_holds_at_the_height_floor_and_fails_one_row_under
the_summary_asks_for_the_fewest_columns_that_hold_its_width
the_first_column_fills_before_a_second_one_opens
the_cell_past_the_initial_rows_rearranges_the_whole_grid
the_columns_fill_the_smallest_square_that_holds_them
redistribute_keeps_the_columns_within_one_cell_of_each_other
a_full_square_grows_a_row_before_it_grows_a_column
twelve_thirteen_and_fourteen_cells_under_each_fill
both_fills_open_the_same_number_of_columns
redistributed_columns_differ_by_at_most_one_taller_first
the_grid_comes_back_down_the_way_it_went_up
the_fifth_column_opens_once_every_column_stands_five_tall
the_square_after_five_grows_the_same_way
every_arrangement_holds_exactly_the_cells_asked_for
nothing_already_placed_moves_until_the_grid_rearranges
initial_rows_below_one_is_treated_as_one
a_cell_that_has_run_out_of_room_takes_what_an_idle_one_is_not_using
room_reaches_a_cell_from_either_side_of_it
a_column_where_everything_fits_divides_evenly
a_cell_gives_up_every_row_its_contents_do_not_need
no_cell_is_pushed_below_what_a_cell_can_be_read_at
no_piece_gets_a_row_past_its_ask_while_a_column_mate_is_short
an_unusable_row_moves_when_its_cell_is_furthest_below_demand
a_freed_row_goes_to_a_mate_whose_next_step_it_completes
a_cell_made_whole_gives_no_row_away
a_freed_row_stays_when_no_mates_next_step_fits
demand_holds_still_until_it_has_a_whole_step_more_to_ask_for
the_focused_cell_wins_the_room_that_does_not_go_round
focus_changes_nothing_while_there_is_room_to_go_round
focus_takes_nothing_from_a_column_that_has_no_room_to_spare
easing_pins_both_ends
```

Former `grid.rs` — port (now in `grid/allocation_tests.rs`, `grid/arrangement_tests.rs`, `grid/tile_grid_tests.rs`, `grid/tests/motion.rs`, `grid/tests/motion_rules.rs` and `grid/tests/summary_depth.rs`):

```text
motion_rules::a_resident_empty_cell_takes_no_row_while_mates_cut
motion_rules::a_command_cell_slides_in_when_its_content_arrives_mid_slide
motion_rules::an_empty_cell_slides_out_across_a_summary_depth_step
motion_rules::every_frame_of_a_key_keeps_the_motion_rules
reused_pids_keep_distinct_tiles_and_focus
a_column_at_the_exact_floor_draws_no_piece_under_it
a_widened_summary_leaves_no_piece_under_the_floor
no_resident_piece_is_under_the_floor_in_flight
a_shrinking_summary_that_widens_keeps_resident_pieces_at_the_floor
a_previous_empty_height_below_the_destination_floor_keeps_ordered_bounds
holds_in_counts_the_headed_summary_depth
a_click_anywhere_in_the_summary_alone_picks_the_summary
focus_returns_to_its_cell_with_the_cells
a_widened_summary_keeps_its_height_and_the_next_column_starts_below_it
a_column_with_no_room_below_the_summary_stops_it
the_summary_widens_only_when_the_setting_is_on
the_arrows_treat_a_widened_summary_as_heading_what_it_covers
an_exact_demand_change_redivides_without_motion
a_shrinking_cell_updates_its_exact_demand_across_a_step_boundary
a_cell_keeps_what_it_grew_to_while_nothing_encroaches
a_cell_hands_its_room_back_once_another_encroaches
a_column_re_dividing_travels_the_way_a_cell_moving_does
a_column_divided_by_demand_covers_its_area_the_way_an_even_one_does
a_column_starts_where_the_one_before_it_ends
a_row_starts_where_the_one_above_it_ends
a_settled_grid_places_one_piece_per_cell
placements_are_the_summary_alone_while_the_cells_are_hidden
the_table_cell_is_never_removed
a_grid_with_no_room_refuses_to_grow
a_cell_changing_column_is_drawn_in_both
a_wrapping_cell_shares_its_column_edges_with_the_cells_that_stay
a_wrapping_cell_leaving_with_its_column_is_squeezed_off_the_edge
a_cell_taking_its_column_with_it_closes_off_the_right_edge
a_cell_leaving_a_column_that_stays_closes_onto_its_floor
adding_the_tenth_live_cell_never_draws_a_survivor_in_a_third_column
a_key_that_changes_slots_and_depth_queues_one_step
no_unchanged_cell_reverses_height_across_one_key
a_key_whose_new_widths_change_demands_still_moves_once
a_departure_uses_recorded_heights_when_its_rows_drop_to_zero
seven_cell_plus_and_minus_start_every_piece_one_way
filled_motion_sources_begin_at_their_settled_heights
mixed_height_changes_move_each_piece_one_way
a_large_height_shrink_moves_one_way
three_snapped_mates_move_one_way
mixed_snapped_height_changes_move_one_way
unchanged_mates_stay_between_their_endpoints_during_plus_and_minus_motion
selected_piece_heights_are_not_reversed_by_absolute_divider_limits
a_shrinking_piece_skips_the_height_that_leaves_a_row_unused
a_resize_during_motion_draws_every_column_inside_the_new_area
returning_to_a_motion_source_area_keeps_the_resized_destination
an_every_other_height_shrink_moves_one_way
a_height_search_reaches_the_nearest_valid_height_each_way
a_painted_optional_row_fills_a_moving_height
a_growing_resident_uses_the_current_frames_whole_row_count
a_widening_resident_uses_its_current_widths_whole_row_count
a_departing_piece_with_no_content_can_collapse
a_moving_piece_snaps_to_a_height_its_content_fills
a_whole_moving_piece_gives_spare_rows_to_a_cutting_mate
a_summary_shrink_starts_filled_and_never_reverses
a_content_change_keeps_residents_at_their_last_filled_heights
a_snapped_height_moves_one_way_through_a_motion
a_column_still_tiles_its_band_while_pieces_snap
two_stepped_mates_both_stand_at_heights_they_fill
arriving_and_leaving_pieces_snap_without_a_floor
a_seven_cell_moving_frame_limits_row_probes
a_settled_frame_measures_each_cell_size_once_and_requests_no_frame
changed_retained_rows_update_the_next_settled_transfer
column_bands_tile_the_area_through_a_transition
pieces_in_a_column_share_their_separators_through_a_transition
motion_endpoints_keep_every_present_cell_in_its_settled_rect
a_motion_frame_applies_unused_rows_to_its_own_arrangement
a_cell_claimed_during_an_opening_stays_in_its_transition_column
a_command_arriving_opens_its_own_cell
a_command_arriving_takes_the_first_empty_cell
the_cells_stand_in_the_order_they_are_demanded_in
re_ordering_leaves_an_empty_cell_where_it_was_made
a_command_leaving_the_middle_moves_the_rest_forward
one_departing_piece_is_in_flight_while_the_grid_closes
a_command_leaving_the_middle_closes_the_grid_in_one_step
a_cell_the_order_overtakes_closes_and_comes_back_in_behind
the_order_changing_cascades_down_the_rest_of_the_grid
no_step_of_a_reorder_draws_two_cells_crossing
a_reorder_leaves_the_cell_count_where_it_found_it
focus_waits_for_a_cell_that_closed_to_let_the_order_through
commands_ending_together_close_one_cell_at_a_time
no_piece_in_a_closing_column_changes_height
a_cell_leaving_a_column_that_stays_slides_off_its_edge
a_middle_departure_starts_on_its_before_rows_between_its_neighbours
removing_refuses_to_close_a_cell_holding_a_command
removing_closes_the_last_cell_that_plus_opened
the_summary_holds_focus_to_begin_with
focus_walks_the_grid_and_stops_at_its_edges
tab_walks_every_cell_and_comes_back_round
focus_follows_a_cell_through_a_closing
focus_falls_back_to_the_summary_when_its_command_ends
focus_stays_on_an_empty_cell_a_command_claims
a_click_inside_a_cell_takes_the_focus_ring
a_click_outside_the_grid_finds_no_cell
minus_takes_out_the_focused_empty_cell
minus_leaves_a_running_command_alone
minus_with_no_empty_cell_changes_nothing
summary_depth::a_summary_one_row_short_takes_it_from_its_mates
summary_depth::a_summary_far_short_still_takes_a_second_position
summary_depth::summary_depth_never_falls_as_the_summary_asks_for_more
summary_depth::no_mate_goes_under_the_minimum_to_feed_the_summary
summary_depth::the_summary_counts_as_the_fewest_cells_that_hold_it
summary_depth::the_summary_gives_a_cell_back_once_it_fits_in_fewer
summary_depth::a_deeper_summary_pushes_every_cell_one_place_on
summary_depth::a_deeper_summary_tiles_its_column
summary_depth::a_capped_summary_places_every_cell_without_an_empty_column
summary_depth::widths_follow_capped_summary_positions_before_the_next_sync
summary_depth::the_summary_never_takes_the_grid_past_fits
summary_depth::the_summary_is_served_before_the_focus_ring
summary_depth::moving_focus_never_changes_the_summary_depth
summary_depth::the_arrows_move_by_position_beside_a_deep_summary
summary_depth::a_click_on_any_part_of_a_deep_summary_focuses_it
summary_depth::adding_under_new_growth_queues_its_summary_depth
summary_depth::cell_numbers_hold_while_the_summary_deepens
summary_depth::deepening_travels_the_way_a_closing_cell_does_in_reverse
summary_depth::a_steady_summary_queues_nothing
summary_depth::a_deeper_summary_widens_over_the_columns_its_depth_opens
a_donor_whose_row_use_jumps_as_it_shrinks_keeps_its_used_content
a_startup_column_with_nonmonotone_row_use_finishes_sharing
a_large_cell_column_measures_each_allocation_once
a_loaned_summary_yields_separators_to_hidden_process_rows
a_summary_separator_uses_a_mates_unpainted_spare_row
nonmonotone_starting_dividers_keep_every_piece_inside_its_band
the_cells_view_keeps_cells_while_the_summary_draws_fewer_rows_beside_them
the_cells_view_returns_while_the_summary_is_cut_beside_them
```

Port notes for the grid test files:
- Floor, summary depth and span, and sticky-allocation tests read the pure target and the explicit retained policy.
- Admission, claim and focus tests inspect stable physical identities and the prepared displayed layout.
- Height tests read current rects after `step`; `HeightSnap` and divider-search fixtures become measured content-fit fixtures, under item 15 for sparse heights.
- The departure-recording test seeds the displayed rect directly; no settled-source snapshot is needed.
- Probe tests assert upper bounds: 29 moving calls and the existing settled measurement budget, with uniqueness kept and `tick() == false` when settled. Exact equality would reject a cheaper implementation.
- "Queues one step" and "queues nothing" tests keep their one-motion and no-extra-motion guarantees without inspecting a queue.
- Changed-column and sequential reorder or departure tests port to item 14. Resize tests keep containment and the rule that stale geometry never returns, without assuming every resize ends motion.
- F054's two regressions keep their protected-content and termination assertions; the two large-cell measurement tests keep their budgets as ceilings; `a_loaned_summary_yields_separators_to_hidden_process_rows` keeps main's separator rule, with process rows still first.
- The view tests keep explicit Cells behaviour: `the_cells_view_keeps_cells_while_…` and `the_cells_view_returns_while_…` keep their Cells assertions, and no ported test asks for a summary-only view that Cells no longer gives (Auto left in phase 20).

`grid/allocation_tests.rs` — retire: `every_queued_state_keeps_surviving_cells_in_an_endpoint_column`. It enumerates `pending` arrangements and checks their endpoint-column assignment; those intermediate queued arrangements no longer exist. Its visible containment protection stays in the port of `adding_the_tenth_live_cell_never_draws_a_survivor_in_a_third_column` and in the live changing-width checks.

Former `draw.rs` — keep (now in `draw/foot_tests.rs` and `draw/rows_tests.rs`):

```text
summary_foot_is_written_only_on_summary_readout_row
summary_foot_leaves_wide_readout_against_right_inset
summary_foot_shares_with_the_short_readout_candidate
summary_foot_is_drawn_whole_or_absent_at_every_width
draw_tile_cell_draws_only_the_readout_on_its_foot_row
the_rows_readout_is_a_whole_candidate_at_every_width
a_cell_with_one_body_row_draws_its_contents_and_no_readout
an_empty_tile_number_uses_the_only_body_row_before_the_readout
```

Former `draw.rs` — port (now in `draw/tests/motion.rs`, `draw/paint_tests.rs`, `draw/rows_tests.rs` and `draw/foot_tests.rs`):

```text
a_cell_lends_no_row_it_must_take_back_within_one_key
a_toast_width_summary_never_draws_compact_with_room_above_its_foot
a_turn_that_grows_content_and_height_draws_no_blank_row
a_summary_hiding_rows_beside_cells_keeps_the_cells
summary_view_shows_the_summary_alone_while_cells_fit
cells_view_shows_cells_whenever_they_fit_at_the_floor
no_view_draws_a_cell_under_its_floor
the_view_holds_through_a_motion
the_summary_shows_no_blank_row_while_a_cell_in_its_column_is_short
a_cell_draws_no_spare_row_while_a_column_mate_hides_rows
reported_unused_rows_go_to_the_most_short_column_mate
a_piece_in_flight_leaves_no_cell_unpainted
a_line_over_an_overlapped_cell_sits_on_the_screen_ground
a_new_column_draws_its_whole_frame_while_its_first_cell_rises
a_piece_entering_the_middle_column_leaves_the_bottom_frame_whole
an_edge_only_piece_adds_no_line_above_the_column_frame
an_arriving_piece_draws_no_line_until_it_has_a_row
a_cell_arriving_at_a_column_foot_leaves_no_line_before_its_first_row
a_content_change_mid_motion_leaves_no_blank_row
a_departed_summary_entry_gives_its_spare_rows_to_cutting_mates_at_once
a_summary_losing_content_during_a_key_gives_its_spare_rows_at_once
an_empty_departing_cell_gives_rows_along_its_slide
a_command_growing_before_a_key_keeps_its_drawn_rows
a_key_claimed_by_a_waiting_command_brings_no_second_empty_piece
no_cell_cuts_a_row_while_a_mate_holds_a_spare_row_it_could_give
a_content_change_keeps_the_motion_clock_with_a_key_waiting
an_arriving_cell_keeps_advancing_when_content_retargets_its_key
a_resize_ends_a_content_retarget_for_good
a_content_retarget_that_changes_columns_keeps_its_clock_and_filled_rows
a_widening_summary_keeps_every_piece_inside_its_moving_band
a_narrowing_summary_keeps_every_piece_inside_its_moving_band
a_wider_summary_discards_a_destination_from_different_columns
a_settled_spare_row_check_rejects_a_donor_beside_a_cutting_mate
a_command_claims_its_empty_cell_without_hiding_a_departing_mate
a_rendered_claim_joins_the_motion_after_an_independent_scan
no_piece_rests_on_a_row_it_cannot_fill_while_a_mate_cuts_content
a_key_that_adds_or_removes_a_cell_slides_its_column
the_first_cell_below_a_summary_slides_in_and_out
the_last_cell_leaves_in_steps_with_its_narrowing_column
startup_cells_leave_no_line_before_their_first_row
a_title_is_on_screen_once_while_its_cell_moves_right_and_columns_stay
a_title_is_on_screen_once_while_its_cell_moves_right_and_a_column_closes
a_title_is_on_screen_once_while_its_cell_moves_left_and_its_column_closes
a_title_is_on_screen_once_while_its_cell_moves_left_and_columns_stay
an_empty_cell_number_is_on_screen_once_while_it_moves_right_and_columns_stay
an_empty_cell_number_is_on_screen_once_while_it_moves_right_and_a_column_closes
an_empty_cell_number_is_on_screen_once_while_it_moves_left_and_its_column_closes
an_empty_cell_number_is_on_screen_once_while_it_moves_left_and_columns_stay
a_departing_cell_keeps_its_title_while_its_border_is_visible
content_glyphs_do_not_cover_ground_only_rows
no_line_is_doubled_through_a_transition
two_cells_entering_a_column_never_double_a_frame_line
a_collapsing_cell_leaves_no_ground_only_row_between_neighbours
sliding_pieces_leave_no_band_row_on_the_screen_ground
a_sliding_arrival_shows_its_nearest_content_row_with_its_first_visible_row
a_fading_cells_readout_fades_with_it
a_cell_cut_at_its_source_never_reads_whole
a_grid_too_wide_for_its_window_draws_the_summary_alone
the_cells_stand_settled_in_the_first_frame_that_holds_them
a_command_refused_in_a_small_window_opens_when_there_is_room
a_command_refused_for_height_opens_when_there_is_room
a_hidden_grid_too_small_draws_nothing
a_closing_column_in_flight_never_draws_a_cell_under_the_floor
a_depth_change_in_flight_never_draws_a_cell_under_the_floor
hidden_grid_draws_no_summary_foot
a_large_cell_frame_measures_each_content_allocation_once
a_shifted_summary_returns_only_the_frame_its_buffer_paints
a_widened_summary_has_no_interior_band_divider
an_entering_tile_keeps_its_title_on_the_last_terminal_row
moving_tiles_keep_title_segments_together_and_inside_their_width
```

Port notes for the draw test files: hand-built `TilePiece` fixtures become one-cell prepared-frame fixtures, and public-grid tests prepare before painting. Buffer text, glyph, ground, title-count, fade, useful-row and floor assertions stay. Clock and source-cache assertions become admission and deadline continuity and latest-target assertions. `a_content_change_keeps_the_motion_clock_with_a_key_waiting` keeps content continuity, with the explicitly queued second key replaced by immediate retargeting. The title-once and crossing tests follow item 14. `a_shifted_summary_returns_only_the_frame_its_buffer_paints` reads the summary frames from the prepared frame. The draw test files retire no test.

Tests added after phase 20, kept with their visible assertions:

| File under `crates/tui_pane/src/tiles/` | Added tests |
|---|---|
| `grid/allocation_tests.rs` | `a_growth_change_during_demand_motion_keeps_every_group_visible`; `a_layout_growth_change_replans_the_motion_endpoints`; `a_floor_change_rechecks_the_painted_widened_summary_endpoint`; `a_floor_change_discards_queued_endpoints_and_measured_motion_heights`; `width_limited_columns_keep_the_exact_resident_capacity`; `summary_redistribution_checks_the_remaining_resident_band`; `width_limited_admission_keeps_resident_motion_within_the_band_budget`; `falling_tree_demand_keeps_the_first_selected_destination`; `narrow_candidate_row_changes_reselect_a_wide_summary` |
| `grid/arrangement_tests.rs` | `a_resize_during_queued_closings_settles_focus_and_keeps_navigation_working` |
| `grid/tests/motion.rs` | `idle_visual_motion_bands_and_title_transfers_need_one_body_paint`; `idle_visual_equal_motion_polls_do_not_consume_the_painted_frame`; `an_action_growth_change_replans_the_motion_endpoints`; `unchanged_motion_frames_reuse_endpoint_admission`; `an_area_change_discards_queued_endpoints_and_measured_motion_heights` |
| `draw/fade_tests.rs` | `idle_visual_appearance_change_requests_a_settled_frame`; `idle_visual_theme_and_transparency_changes_need_a_body_paint` |
| `draw/paint_tests.rs` | `hidden_summary_paint_and_polls_keep_the_same_empty_drawing` |
| `draw/rows_tests.rs` | `selected_rows_readout_checks_bounds_and_preserves_count_text`; `transparent_tile_readout_fades_toward_theme_without_painting_ground`; `focused_tile_tint_covers_short_rows_empty_rows_and_readout`; `selected_rows_readout_uses_measured_content_for_fit_ink`; `an_erased_inactive_readout_that_fits_does_not_cover_its_row` |

New tests:

| Item | Test | Behaviour pinned |
|---|---|---|
| 1 | `a_second_opening_starts_at_its_own_opening_height` | A cell opens during an existing entry; the new cell starts small, and the existing entry neither restarts nor shrinks. |
| 2 | `a_mid_motion_admission_has_its_own_720ms_clock` | A command admitted late in another motion; its first displayed allocation and its full admission lifetime are its own. |
| 3 | `a_leaving_cell_survives_a_summary_depth_change_until_its_slide_ends` | Every rendered frame across the depth change; no disappearance from more than two rows. |
| 4 | `ordinary_residents_change_by_at_most_three_rows_per_frame` | Resident heights and transfer classes on the production frame cadence. |
| 5 | `content_shrink_transfers_rows_atomically_to_cutting_residents` | Donor and every recipient change on the observation frame; the unused remainder follows the clock. |
| 6 | `an_arrival_free_key_reaches_one_target_without_height_reversal` | `+` and `-`, both fills, a settled start, no arrivals; one completion and heights that move one way. |
| 7 | `resizing_every_frame_keeps_the_displayed_grid_inside_the_area` | Shrink, grow and return through column and depth changes; no geometry escapes and no stale endpoint returns. |
| 8 | `minus_preserves_staying_content_until_departure_releases_rows` | Staying process rows stay visible; summary growth takes no rows the departure has not released. |
| 9 | `a_summary_gives_its_unused_row_on_the_arrival_frame_while_a_piece_leaves` | The real draw path, a leaving cell two rows high, a new summary project; on the arrival frame the buffer has no blank row while a project is hidden. |
| 10 | `a_job_crossing_columns_draws_each_content_row_once_per_frame` | Both directions and a closing column, with distinct heading, ancestry and process markers; one provider paint and at most one copy of each row. |
| 11 | `minus_during_motion_retargets_every_cell_within_one_frame` | Target publication on receipt, geometry continuity at `dt = 0`, and the first movement on the next frame. |
| 14 | `a_cell_changing_column_closes_then_opens_as_one_rect` | Both directions and a closing column; one rect per frame, closing over the first half and opening over the second, no overlap or crossing, content, title and focus kept. |
| 15 | `a_resident_moves_straight_to_a_filled_height_more_than_three_rows_away` | The move is one named transfer and no frame shows a blank row. |
| 16 | `an_entering_cell_keeps_its_height_while_its_band_has_room` | A retarget below its displayed height; it holds while the band has room, moves at the resident rate when it does not, and its admission clock never restarts. |
| 17 | `a_longer_parent_pid_widens_its_column_once_during_a_motion` | `MotionColumnWidths` widens a column once, when the longer pid arrives; every other offset is constant across the motion. |
| 17 | `motion_column_widths_narrow_only_on_the_settled_frame` | Rows that lose their longest pid keep every offset until the settled frame. |
| 18 | `an_allocated_out_long_row_keeps_cpu_and_mem` | Columns fit the rows the final allocation draws; a long row it leaves out removes no column. |
| opening | `a_new_opening_starts_at_its_first_drawable_allocation` | No frame draws an opening border-only; its first visible frame holds a content row, and a `+` vacant cell keeps its own opening. |
| departure | `a_leave_runs_text_fade_and_shrink_on_one_timing` | Injected time at a 3 s and a 30 s configured fade; text alpha and height reach their ends on the same frame, and the cell is erased on the frame it can no longer draw a content row. |
| departure | `a_finished_jobs_cell_leaves_instead_of_staying_an_empty_box` | When a command's job finishes, its cell slides out and is removed; no frame leaves it as a bordered box with no name. |

Also added: `replaying_the_same_events_and_dt_produces_identical_frames`, `painting_twice_changes_no_layout_state`, `an_unchanged_scan_does_not_extend_travel`, and `claiming_an_entering_vacant_cell_preserves_its_identity_and_admission`.

**Files:**
- `crates/tui_pane/src/tiles/grid/{mod,demands,pieces,rows,heights,motion,arrangement,visual,selection,tile_grid,layout,allocation,bands}.rs`, `crates/tui_pane/src/tiles/grid/{allocation_tests,arrangement_tests,tile_grid_tests}.rs`, `crates/tui_pane/src/tiles/grid/tests/{mod,motion,motion_rules,summary_depth}.rs`, `crates/tui_pane/src/tiles/draw/{mod,contents,fade,rows,foot,measurements,paint}.rs`, `crates/tui_pane/src/tiles/draw/{fade_tests,rows_tests,foot_tests,paint_tests}.rs`, `crates/tui_pane/src/tiles/draw/tests/{mod,motion}.rs`, `crates/tui_pane/src/tiles/constants.rs` — the motion code, the painter, `TileGroupTitle` and their tests.
- `crates/tui_pane/src/tiles/target_layout.rs`, `crates/tui_pane/src/tiles/motion.rs` — new: the pure target layout and the step.
- `crates/tui_pane/src/tiles/settings.rs` — the ripple step settings, which the new model drops.
- `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — the additive preparation API and the `TileGroupTitle` export.
- `crates/cargo-tile/src/render.rs`, `crates/cargo-handler/src/render/mod.rs`, `crates/cargo-handler/src/render/role_title_tests.rs` — the render callers, which prepare a frame before they call `draw_tile_grid`, cargo-handler's `group_title` and its title style regression (item 13), and cargo-tile's `MotionColumnWidths` and allocation-aware column fit (items 17 and 18).
- `crates/cargo-tile/src/render/frame_preparation.rs` — preparation inputs, retained tables, allocation decisions and row measurements.
- `crates/cargo-tile/src/render/command_cache.rs` — retained command widths, wrapped text, row cells and table layouts.
- `crates/cargo-tile/src/render/cell_paint.rs` — paint inputs, destination validation, command visibility and paint replay.
- `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/census/cpu_observation.rs` — item 19's CPU frame stamp pairing and its test.
- `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — the whole-frame test helpers, where the preparation step reaches them; the tile test files named above and both apps' `render.rs` tests that call `draw_tile_grid` move with it.
- `crates/tui_pane/CHANGELOG.md` — one Fixed line, one Changed line for the group title, and one Added line for the preparation API.

**Workers:** 1 writer + 1 tester — the items sit in one motion path, so one seat writes.
- `impl` — every source file above: the motion and painter code, the exports, both apps' render callers and their test callers, cargo-tile's column widths (items 17 and 18), item 19's CPU frame stamp pairing in `app.rs` and `census/cpu_observation.rs`, and the `tui_pane` changelog; lists each new test that reaches 0.8 s in the phase report.
- `test` — no source file; owns the live fixture, capture and cost lanes. Before any repair it replays the live cases on the phase 35 checkpoint build and writes each item's fixture roles and expected heights for `impl`, item 22's distinct-row provider and its expected row per frame included; then it runs the live capture tools and the per-frame cost comparison on each candidate build.

**Constraints from prior phases:** Phase 32b: `PreparedTables::prepare` keeps cell paintings across roster revisions, and `CellPaintSource::matches` validates each one against its sources. A geometry change changes those sources, or the cell keeps a stale painting. Unchanged displayed values keep their reuse. Phase 32 and Phase 35: the shared plan carries the `+N more` count reservation, the hidden totals and `ProcessRowVisibility`. Phase 14's ruling holds: a piece that is entering or leaving slides, and the spare-row rule binds cells that are neither, and every settled frame. Everything Phase 14's live check met stays met: the emptied cell on the seventh key stands 0 frames above its floor, the summary shows 0 blank rows, no empty cell leaves from its full floor height, one piece per `+`, 0 band escapes. No new row probe or row measurement per moving frame; the seven-cell moving budget stays 29, and speed stays within 1 ms of the Phase 13 build. Three repair rounds in Phase 14 each fixed their fixture and moved the defect to a neighbouring live state (memory `attempts-tile-phase14-motion-rows`): a repair is proved on the live capture before it closes. Phase 37 runs after this phase by the showrunner's landing call and measures the suite it ships. This phase runs after Phases 19 and 20, and after Phase 35, which changes `render.rs`'s per-frame caches and table walk and may change how `draw_tile_grid` decides a frame (its items 5 to 7); Phase 19 changes the summary's spare-row taker, its separators and the row order (its items 11, 14 and 15): replay every case on the phase 35 checkpoint build before repairing, and keep Phase 19's shared measurements. Phase 24 trims the step queue this phase removes (Phase 22 items 1 to 4; Phase 28 did not land, so these trims and this test are not in the tree and this phase writes the test new), so a command that ends mid-backlog leaves the grid on the next motion; its test `a_command_that_ends_mid_backlog_leaves_the_grid_on_the_next_motion` is a binding regression here: it passes on the new model with its visible assertion unchanged, and its fixture is ported only where it names the queue. The fixture baseline is the phase 35 checkpoint; earlier captures stay as evidence only. Their absolute roots, under `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad`: shots in `ph19`, `ph19b`, `ph19c`, `ph19d` and `ph19e` (each with `shots/`); captures in `p19`, `p19b`, `p19c`, `p19d` and `p19e`, and the motion captures `probe12/keepcap_p19`, `probe12/keepcap_p19b`, `probe12/keepcap_c_p19` and `probe12/keepcap_c_p19b`, with their capture scripts `probe12/cap7c_p19b.sh` and `probe12/cap6_p19b.sh`. `TileCells::readout_rows` and `TileCells::content_extent` keep their answers on the final allocation in the prepared frame; painting triggers no new measurement. Items 5 and 8 may already be partly met (`draw/tests/motion.rs:442` `a_departed_summary_entry_gives_its_spare_rows_to_cutting_mates_at_once` and `draw/paint_tests.rs:525` `a_summary_losing_content_during_a_key_gives_its_spare_rows_at_once` cover immediate summary donation): repair only what a replayed live failure shows. The immediate transfers items 5 and 15 permit are each one event for every piece they touch: the resident that gives or moves rows and each mate that takes them may change by more than 3 rows in that frame; the Goal, the tests and the capture tool's counters class them the same way, and the jump gate excepts them. Type design: `HeightSnap::previous_height -> Option<u16>` (`grid/heights.rs:90`; `HeightSnap` at line 21) folds disabled probing into missing history, and `open_waiting_commands_in_current_motion` (`grid/tile_grid.rs:1635`) stands a new piece in at height 1; the repair names admission time and height history as states at that boundary, with no bare `Option` or height sentinel as the answer. The shared fixture is `grid/tests/motion_rules.rs:551` `every_frame_of_a_key_keeps_the_motion_rules`. This phase does not edit `.config/nextest.toml`: it lists each new test that reaches 0.8 s, and Phase 37's measurement and exact-name override place it. `draw_tile_grid` keeps its public signature: it becomes a read-only painter, and a frame is prepared before it through an additive preparation API, so drawing writes no layout state. Phase 16 kept `group_title`'s signature; this phase's change to it is the approved exception the Delegation Context names. Toast placement's observable geometry holds: `a_shifted_summary_returns_only_the_frame_its_buffer_paints` (`draw/paint_tests.rs:74`), cargo-tile's `a_toast_leaves_the_summary_foot_drawn` and `toasts/render/drawing.rs`'s `a_toast_area_avoids_only_crossing_frames_and_keeps_the_larger_region` pass with their assertions unchanged. The config reader still observes a legacy `auto` view: `crates/tui_pane/src/app_config/tile_view.rs`'s `a_saved_auto_view_loads_as_cells_and_saves_as_cells` stays. Phase 17 rebuilt `unchanged_mates_stay_between_their_endpoints_during_plus_and_minus_motion` to sample its stable frames by integer height steps (test helpers `endpoint_painted_rows`, `single_column_height_steps`, `endpoint_rows_drawn`) and to measure every asserted frame, the last included, through the frame preparation; an exhaustive comparison over 1001 frames matched, and it runs in 0.056 s. Its port keeps both: per-frame measurement through the new step, and step sampling instead of a sweep of every raw progress value. Phase 26 shortened more grid and draw fixtures the same way: they sample only both sides of each change to a fixed provider's integer geometry inputs (`distinct_motion_steps_for_test`, `grid/tile_grid.rs:2758`), publish the fixed provider's content change once before they measure (`fixed_motion_drawing` and `measured_motion_drawing`, `draw/tests/mod.rs:618` and `:634`), and measure each asserted frame. Among them are `a_key_whose_new_widths_change_demands_still_moves_once`, `arriving_and_leaving_pieces_snap_without_a_floor`, `a_turn_that_grows_content_and_height_draws_no_blank_row`, `a_cell_lends_no_row_it_must_take_back_within_one_key` and `a_cell_arriving_at_a_column_foot_leaves_no_line_before_its_first_row`. Their ports keep their assertions, publish each fixed provider change once, and measure each asserted frame. A port that samples another way states in the phase report why its frames cover every geometry change under the stateful step, where a cell's displayed rect depends on the frames before it, and each test stays under 1 s. Phase 27's CPU frame stamp is tied to a drawn frame: `App::draw` (`crates/cargo-tile/src/app.rs` ~314) calls the test-only `cpu_observation::publish_frame()` (`crates/cargo-tile/src/census/cpu_observation.rs`), which writes the OSC 777 `cargo-tile-cpu` stamp to stdout for the scan `drain_scans` accepted. Item 19 keeps that pairing: a suppressed draw writes no stamp, each frame that is drawn (the status frames included) carries the accepted scan's stamp, and the stamp output schedules no redraw deadline. The CPU reader test and the five `reader_cpu_oracle_*` tests pass unchanged. `PreparedFrameTables` retains table products; `draw_tile_grid` still measures demands, changes grid layout and records painted geometry. Geometry preparation consumes the retained table products; add no other frame cache and do not restore repeated command preparation. Include displayed geometry, motion policy and retained column widths in preparation. Publish geometry before painting. Preserve unchanged table and command preparation. Reader PTY tests and settled helpers do not prove animated leave, pause or retarget behavior. Use injected-time buffer fixtures and production-motion captures for those acceptance gates. Keep `TileSettings::min_tile_height` and the corresponding allocation-rule value as `NonZeroU16`. Keep `TileGrid::set_min_tile_height(u16)` and its existing floor behavior.

**Forward edits from Phase 46 (director, 2026-11-07):**
- The expanded view uses grid travel (`tui_pane/src/tiles/grid/tile_grid.rs:2559`, `draw_cell_view`). The one target layout includes expand and collapse. The prepared frame keeps reversal, the collapse target on that frame, covered-grid preparation and toast protection (`render.rs:430`).
- Keep the Phase 46 motion tests green. Compare total motion CPU and frame counts for expand, collapse and ordinary travel against the pre-phase reader.

**Forward edits from Phase 49 (director, 2026-11-07):**
- Phase 49 footer contract: the summary foot is `SummaryFoot::WithOptionalSuffix` (`tui_pane/src/tiles/draw/foot.rs:17`), prefix `mem`, suffix `disk`. The unit is `G`. Memory has one decimal. Disk has one decimal below 10G and is whole from 10G. At a narrow width the foot keeps `mem` and drops the disk suffix and its gap. `FreeDiskSpace::Unavailable` (`census/scan/refresh.rs:143`) omits the disk value and its gap, with no user remedy.
- Binding: a disk or summary change reaches both preparation paths (Grid and Expanded).
- `SummaryFootPreparation` (`render/frame_preparation.rs`) retains the foot by memory, rounded disk text and theme. `PreparedPaneFrame` joins it to the independently retained tables. A table change keeps this boundary. `disk_text_changes_reuse_table_preparation_and_update_only_the_foot` and `an_expanded_summary_updates_its_disk_footer_after_scroll_and_resize` pass unchanged. `SummaryFootPreparation` is the one permitted exception to this phase's frame-cache ban.
- Item 19's idle gate assumes unchanged displayed disk text: `drain_scans` (`terminal.rs:296`) also requests a frame when the rounded disk text changes. Add an injected-scan test: a shown disk change updates the foot in each view, restarts no motion and rebuilds no table.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` once, then `verify.sh test` for `tui_pane`, `cargo-tile`, `cargo-handler` and `cargo-port`: green, no test at a second or more.
- Live, controlled workload, two rounds, read with the Phase 14 tools: command cells at once 0; pieces vanishing from over 2 rows 0; resident jumps over 3 rows 0 outside the named transfers of items 5 and 15; binding spare-row frames 0 under item 5's ruling; on keys free of arrivals 0 reversals and one motion per key; band escapes 0 on the changing-width capture.
- Speed on a quiet machine: moving and settled medians within 1 ms of the Phase 13 build.
- Shots of the motion keys and a design check by a fresh helper.
- The live frame check against the phase 35 checkpoint build reads no worse on every gate, and per-frame cost on natedev is no more than that build's.
- Item 17, in the unit director's capture: every column offset is constant across each motion's frames except a widening, which happens at most once per column per motion.
- Item 12, in the unit director's capture: 0 reversal flags and one motion on every key that lands within 0.15 s after a command arrives or claims a cell.
- Item 19: a test that counts drawn frames over an idle stretch, with an injected clock, asserts zero beyond the scheduled once-a-second status frames, and a `CARGO_TILE_FRAME_LOG` reading of an idle live tile shows no frames beyond those status frames. `an_idle_draw_writes_no_cpu_frame_stamp`: a suppressed draw writes no OSC 777 `cargo-tile-cpu` stamp, a drawn status frame carries the accepted scan's stamp, and the stamp adds no redraw deadline; `reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` and the five `reader_cpu_oracle_*` tests pass unchanged.
- Item 21: a frame-stepped test proves a row is drawn on the first frame its rect is inside the tile's current height.
- No `pending`, `Transition.source`, `motion_heights`, `retargeted_destination` or `DestinationDepthStatus` remains, and no draw path writes layout state; `draw_tile_grid`'s signature is unchanged.
- `TileCells::group_title` returns no bare `Option`, and the existing tile render tests pass without edits to the titles they assert; `tile_title_suffixes_use_showrunner_and_unit_role_colors` passes with its role-suffix assertion and the new session-ink assertion.
- Item 22: `column_crossing_content_keeps_its_row_identity` fails on a fallback that lays content out again at its clip, and passes on the new model.
- The three toast placement tests and `a_saved_auto_view_loads_as_cells_and_saves_as_cells` pass unchanged; the painter records no summary frames.
- Every test the Test map keeps passes unchanged; every ported test keeps its visible assertions; every new test passes, `a_finished_jobs_cell_leaves_instead_of_staying_an_empty_box` included; Phase 22's `a_command_that_ends_mid_backlog_leaves_the_grid_on_the_next_motion` and `a_command_hidden_mid_run_leaves_on_the_next_frame` pass with their visible assertions unchanged.
- Probe tests hold the ceilings: at most 29 row-measurement calls on a seven-cell moving frame, the existing settled budget, and no measurement caused by painting.
- Cost: `test` compares the byte-identical candidate against the phase 35 checkpoint build on a quiet natedev, over event frames as well as steady moving and settled frames; per-frame cost is no higher, and the Phase 13 1 ms gate holds.
- The map covers all 264 current grid and draw tests. Retain each visible assertion. Port only fixtures that depend on replaced geometry or motion state. Name every retirement and its retained behavioral coverage.
- The unit director's notice lists each retired grid test with its current module path and its reason.

### Phase 37 — Slow tests start first  · status: todo

**Depends on:** Phase 21, Phase 34, Phase 36, Phase 39, Phase 40, Phase 43b, Phase 43e, Phase 43f, Phase 49

#### Work Order

**Goal:** On each machine, every non-reader test observed at 0.8 s or more in any measurement run starts before every faster non-reader test in a full `cargo nextest run`; the eight reader scenarios keep their existing priority and one-at-a-time rules. The named slow tests were shortened in Phase 17; this phase only orders the rest, and rewrites no test except one: Spec item 6 shortens cargo-port's watcher test (`spawn_watcher_thread_keeps_watcher_guard_alive_until_shutdown`) when its quiet time on natedev is a second or more.

**Spec:**

1. **Measure before.** On each machine run the suite CI runs, `cargo nextest run --all-features --workspace --exclude cargo-mend --tests`, three times with JUnit turned on from outside the repo config: a file `junit.toml` holding `[profile.default.junit]` / `path = "junit.xml"`, passed as `--tool-config-file tile-fixes:<absolute path>/junit.toml`. Each `testcase` then carries its start `timestamp` and `time`. Every run writes the same `target/nextest/default/junit.xml`, so before the next run copy it to `<machine>-<before|after>-<n>.xml` (machine `natedev` or `mac`, `n` 1 to 3, or 4 to 6 for a repeated after batch) in the delegate session directory; the Mac lane copies each file there over `scp` as it finishes, so every run's evidence survives. Record the wall time of each run and, per test, its slowest time. On the Mac work in a clone outside `/tmp`, reached with `ssh mac`: `git clone git@github.com:natepiano/cargo-liner.git /Users/natemccoy/rust/cargo-liner-measure` if it is absent, then before every batch of runs `git -C /Users/natemccoy/rust/cargo-liner-measure fetch origin remove-running-section-tile`, `git -C … checkout --detach FETCH_HEAD`, and confirm `git rev-parse HEAD` equals the phase tip on natedev and `git status --short` is empty. (`/Users/natemccoy/rust/cargo-liner` is the user's checkout on `main`; leave it alone.)
2. **The slow set** is every test at 0.8 s or more in any of the six runs, less the eight reader scenarios the config already orders. Known members to confirm by name with `cargo nextest list`: in `cargo-tile::unit_tests`, `progress::capture::…::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`, eight in `shim_registration::wire::`, two in `hook::`; two each in `cargo-tile::shim_modes` and `cargo-tile::cli_lifecycle`; in `cargo-handler`, `census::codex::…::reads_threads_by_id_whoever_started_them` and `…::reads_interactive_threads_created_in_the_span`; in `tui_pane`, `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw`.
   Also from phase 20's final suites: `tui_pane` `tiles::draw::tests::startup_cells_leave_no_line_before_their_first_row` (1.199 s natedev) and `tiles::grid::tests::a_key_whose_new_widths_change_demands_still_moves_once` (1.167 s natedev, 1.734 s Mac); cargo-tile render tests crossed a second only on a loaded Mac run. Also known (showrunner, 2026-10-16, reported by handler-enh): `tui_pane` `a_toast_width_summary_never_draws_compact_with_room_above_its_foot`, 1.1 s, arriving on main with handler-enh's phase 2 merge; confirm it by measurement like the others.
3. **One override.** Append to `.config/nextest.toml` one `[[profile.default.overrides]]` with `priority = 80` and a `filter` that names each slow test exactly: `binary_id(<id>) & (test(=<name>) | …)` per binary, joined with `|`. No regular expressions, so a test added later is not swept in by accident. A comment above it says what the list is, the threshold, the date measured, and how to re-measure. nextest resolves each setting from the first override that sets it, so the two existing priorities (100, 90), the `cargo-tile-readers` group and both `slow-timeout` entries keep working unchanged; the new override sets `priority` alone and names none of the reader scenarios.
4. **Measure after**, the same three runs per machine. A non-reader test that crosses 0.8 s in an after run and is not in the filter joins it, and the three after runs repeat once on both machines; a second newcomer is reported in the summary rather than chased.
5. **Two reader scenarios, as the earlier phases leave them.** `shim_registration::reader_scenarios::reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` and `reader_keeps_parent_family_across_its_only_childs_source_switch`. Phase 26 stopped the Mac reader's startup scan of the build artifact folder and changed the CPU check, Phase 27 lands the strict CPU check, and Phase 43h's final uninstrumented suites put both tests below 1 s; Phase 33 repairs only a timing that its own accounting changes cause. This phase edits neither test: it measures both in the six runs, as part of the suite those phases ship. Where either takes a second or more on a quiet natedev, record its measured time and cause in the summary and send the showrunner a proposal.
6. **Three more tests over a second under load.** `tiles::draw::tests::a_toast_width_summary_never_draws_compact_with_room_above_its_foot` (tui_pane, 1.19 s and 1.05 s at load 43) and cargo-handler `census::codex::tests::reads_threads_by_id_whoever_started_them` (1.53 s) and `reads_interactive_threads_created_in_the_span` (2.27 s) passed a second in Phase 18's suite runs under heavy load; Phase 18 does not touch them, and the two cargo-handler tests took 0.13–0.14 s in a later run. Measure each in the six runs; for any at a second or more on a quiet natedev, measure its cause, record it in the summary, and send the showrunner a fix proposal, as item 5 does; none of their files is in this phase's Files. Also cargo-tile's `shim_registration::shared_capture::staged_executable_installs_toolchains_and_cleans_up` (1.158 s and 1.321 s at 1-minute loads of 22 to 47 in phase 19's repair gates), under the same rule. Also cargo-port's `watcher::runtime::tests::spawn_watcher_thread_keeps_watcher_guard_alive_until_shutdown` (`crates/cargo-port/src/watcher/runtime.rs`; 1.121 s at a 1-minute load of 248; showrunner routing, 2026-10-13: cargo-port has no other owner in this production): timed on a quiet natedev, shortened in this phase when it takes a second or more there, otherwise its quiet time is recorded and it closes as load. Also, from phase 19's repair round 5 at 1-minute loads of 49 to 62: cargo-tile `render::tests::every_command_cell_shows_a_process` (1.057 s), `shim_registration::wire::matching_relative_directories_keep_their_absolute_paths_and_writer_homes` (1.025 s) and `shim_registration::wire::json_arguments_preserve_both_quiet_spellings_after_separator` (1.134 s), and at loads of 48 to 72 `shim_registration::reader_scenarios::reader_accepts_live_writer_under_different_locale_and_timezone` (1.278 s); time them the same way.

**Files:**
- `crates/cargo-port/src/watcher/runtime.rs` — the watcher test in Spec item 6, only when its quiet time is a second or more.
- `.config/nextest.toml` — one override and its comment. In scope for this unit by the showrunner's word (2026-10-08), though the Units row does not list it.

**Workers:** 1 writer + 1 tester — one configuration owner and two measurement lanes on a byte-identical candidate.
- `impl` — owns `.config/nextest.toml` and, when Spec item 6 shortens it, `crates/cargo-port/src/watcher/runtime.rs`; publishes the `sha256sum` of every changed candidate file on the board after each edit, and runs every natedev before and after run; checks every exact filter name. It also owns Spec items 5 and 6's quiet evidence: it times each named test on natedev with the 1-minute load below a quarter of the cores, records that load beside each time, and gives each test a disposition.
- `test` — no source file; copies every changed candidate file to the Mac clone after the checkout (`scp` over `ssh mac`: the config into `/Users/natemccoy/rust/cargo-liner-measure/.config/nextest.toml` and, when it changed, the watcher test file into `/Users/natemccoy/rust/cargo-liner-measure/crates/cargo-port/src/watcher/runtime.rs`), verifies each file's sha256 against the board before both after-run batches, runs every Mac run, and hands `impl` the named JUnit files, the slow set and the start order. The before runs use the clean phase tip.

**Constraints from prior phases:** This phase runs after Phases 1 to 36, every phase planned to add tests or change shared grid code, so its timings measure the suite those phases ship. It runs last by the showrunner's packaging call (2026-10-12), after Phase 36's last checkpoint when that phase lands in several, so its override names the suite Phase 36's motion rewrite leaves. Phase 36 lists each new test that reached 0.8 s in its report; this phase confirms each by name and by measurement. Phase 17 shortened several of item 2's known members, and Phase 36 replaces most grid and draw tests, so item 2's known list is confirmed against the six runs and `cargo nextest list`, never assumed: a known member under 0.8 s in every run stays out of the filter. Exact-name discovery, the reader priorities and group, and both timeouts hold as item 3 states. Phases 1 to 16, 18 to 20, 23 to 25 and 27 added render tests to `tui_pane` and `cargo-tile`; each gate asked for under a second for its own tests, and this phase's line is 0.8 s, so the six measurements decide whether any of them joins the slow set. Seen over a second during Phase 8's runs under load: `tui_pane` `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw` (1.52 s) and several reader and `shim_modes` scenarios.

**Forward edits from Phase 46 (director, 2026-11-07):**
- Measure the suite that holds Phase 46's tests.

**Forward edits from Phase 49 (director, 2026-11-07):**
- Run after every remaining phase that changes tests, 39, 40 and 43f included, so the measurement covers the suite that ships. Phase 43e replaces parked Phase 43c.
- The priority-100 override holds two readers: source-switch and `reader_scenario_sigkill_ends_its_waiting_writer_group` (Phase 49 slot, `.config/nextest.toml:22`). Without the slot the SIGKILL test took 0.950 s and 1.069 s on natedev. The slot costs about 1.5 s of Mac suite time.

**Acceptance gate:**
- `bash ~/.claude/scripts/direct/verify.sh lint cargo-tile` once, after the config edit: green.
- When `crates/cargo-port/src/watcher/runtime.rs` changes: `bash ~/.claude/scripts/direct/verify.sh lint cargo-port` once, then `... test cargo-port` green, and `spawn_watcher_thread_keeps_watcher_guard_alive_until_shutdown` passes under a second on a quiet natedev.
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, both priority-100 readers (source-switch and SIGKILL) run one at a time before the priority-90 group, in either order, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.
- The summary gives each test named in Spec items 5 and 6 its quiet natedev time and load, and for any at a second or more, its measured cause, its floor and the fix proposal sent to the showrunner.

### Phase 48 — The p and h shortcuts show with the other global shortcuts  · status: done

**Depends on:** none (it named Phase 39 only for file order; Phase 39 has not started)

#### Work Order

**Goal:** The status bar does not show `p process_tree` or `h arguments` in its bottom-right corner. Both shortcuts show with the other global shortcuts, in the `?` shortcuts list. User request, 2026-11-02, through the showrunner.

**Spec:**

1. Remove `p process_tree` and `h arguments` from the status line. `render.rs` adds them through `StatusLineGlobal::app(AppGlobalAction::ProcessTree)` and the arguments entry beside it.
2. Show both shortcuts in the global shortcuts list, with the labels and descriptions in `app_global_action.rs`.
3. The `p` and `h` keys keep their current behavior.
4. Change the status-line tests that expect the two entries. Each Assertion-Change line names the user request as its reason.

**Files:**
- `crates/cargo-tile/src/render.rs`
- `crates/cargo-tile/src/app_global_action.rs`

**Acceptance gate:**
- `verify.sh lint cargo-tile` and `verify.sh test cargo-tile` green; no new test takes 1 s or more.
- A live shot shows the status bar without the two entries, and the `?` list with both.

### Phase 49 — The summary tile shows free disk space in GiB after mem  · status: done

#### As-built

- The summary foot reads `mem 0.4G  disk 302G`. The unit is `G`. Memory has one decimal. Disk has one decimal below 10G and is whole from 10G. `gibibytes_label(bytes, GibibytePrecision)` formats both values.
- `FreeDiskSpace` (`census/scan.rs`) holds the available bytes or `Unavailable`. `FreeDiskSpace::read` calls `statvfs` on each census refresh (0.61 us per read). The value travels through `terminal::Scan`, `drain_scans`, app state and both summary preparation paths (Grid and Expanded).
- `FreeDiskSpace::Unavailable` omits the disk value and its gap. A failed read has no user remedy.
- `SummaryFoot::WithOptionalSuffix { prefix, suffix }` (`tui_pane/src/tiles/draw.rs`) keeps `mem` and drops the disk suffix and its gap when the value cannot fit.
- `SummaryFootPreparation` (`render/frame_preparation.rs`) retains the foot by memory, rounded disk text and theme. `PreparedPaneFrame` joins it to the independently retained tables.
- `drain_scans` requests a frame only when the rounded disk text changes. Idle tile CPU stays inside the spread of 3 idle pairs (median +0.017 pp, spread 0.067 pp).
- `.config/nextest.toml` puts `reader_scenario_sigkill_ends_its_waiting_writer_group` in the priority-100 exclusive-slot override beside the source-switch reader.

**Files:**
- `crates/cargo-tile/src/census/scan.rs` — `FreeDiskSpace` and its `statvfs` read.
- `crates/cargo-tile/src/terminal.rs`, `crates/cargo-tile/src/app.rs` — disk transport, app state and the repaint decision.
- `crates/cargo-tile/src/render.rs` — `gibibytes_label`, `GibibytePrecision` and the foot buffer tests.
- `crates/cargo-tile/src/render/frame_preparation.rs` — `SummaryFootPreparation`.
- `crates/cargo-tile/src/constants.rs` — the foot labels.
- `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/CHANGELOG.md` — the optional-suffix foot.
- `.config/nextest.toml` — the SIGKILL reader in the exclusive slot.

**Binds later work:** a disk or summary change reaches both preparation paths. `disk_text_changes_reuse_table_preparation_and_update_only_the_foot`, `an_expanded_summary_updates_its_disk_footer_after_scroll_and_resize` and `disk_labels_use_whole_gibibytes_from_ten_and_tenths_below` pass unchanged. The priority-100 override holds two readers.

**Gotchas:** without its exclusive slot the SIGKILL reader test took 0.950 s and 1.069 s on natedev (0.397-0.664 s with it). The slot costs about 1.5 s of Mac suite time and none measurable on natedev.

**Ruled out:** a `GiB` label and a `free` label (showrunner ruling: `G` and `disk`); dropping the SIGKILL exclusive slot (it breaks the 1 s rule on natedev).

### Phase 54 — The reader SIGKILL scenario passes in a full Mac run  · status: done

#### As-built

- `assert_killed_scenario_ends_writer_group` names its waits for writer creation, writer readiness and writer-group exit.
- Each wait uses the scenario’s existing deadline through `named_wait`.
- The scenario retains its checks for a live writer and child, SIGKILL termination, and writer-group exit.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario.py` — the three named waits and unchanged cleanup paths.

**Binds later work:** “No cargo-tile test takes a second or more” preserves these waits while reducing scenario cost.

**Gotchas:** Host load can delay process creation and guardian cleanup. The test duration target does not impose a separate short deadline on each wait.

**Ruled out:** Fixed subsecond writer-readiness and writer-exit limits.

### Phase 50 — A CI nextest run shows its progress meter  · status: done

**As built** (checkpoint `2a55fa9c2`, merged as main `431ecb2e8`, 2026-11-03): no code change. The CI nextest meter was missing because an old cargo-tile reader was not restarted; a reader built from main draws it. The test `a_captured_ci_nextest_draws_its_meter_without_lending_it_to_a_child` pins that a captured CI nextest run draws its own meter and its child rows borrow none.

### Phase 51 — A reader scenario ends its fake cargo processes  · status: done

**Depends on:** none. It starts when the showrunner lifts the cargo pause for it.

#### Work Order

**Goal:** A reader scenario leaves no fake cargo process when it ends, fails or is killed. From the builds showrunner, 2026-11-02: a fake `cargo build probe-first-.tmptzAS15` from `reader_scenario.py` ran on the Mac for 54 hours and showed in the user's tile grid. It started on 2026-10-31 at 16:23 EDT. Its log reads "Blocking waiting for file lock on build directory".

**Spec:**

1. Find how the scenario ended without its cleanup. The `finally` block releases writers through trigger files, and the writers run in their own sessions (`start_new_session=True`). A killed scenario runs no `finally`.
2. Make each writer's process group end when its scenario ends, also on SIGKILL. Fix it where the writers start, not with a sweep after the fact.
3. A normal end keeps its current release order and its time.
4. Add a test: kill the scenario with SIGKILL while a writer waits; no writer process stays. The test takes less than 1 s.

**Files:**
- `crates/cargo-tile/src/shim_registration/reader_scenario.py`
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs`

**Acceptance gate:**
- `verify.sh lint cargo-tile` and `verify.sh test cargo-tile` green on natedev and the Mac; no new test takes 1 s or more.
- After the Mac pass, no process from a scenario folder stays on the Mac.

### Phase 53 — A lifecycle test leaves no process running  · status: done

**Depends on:** none

#### Work Order

**Goal:** `install_prints_a_completed_toolchain_before_a_later_lock_finishes` leaves no process when it ends. On the Mac, the Phase 51 run on 2026-11-05 reported this test as leaky at 0.637 s. The showrunner put this item in the plan on 2026-11-05.

**Spec:**

1. Find what the test leaves running after it returns.
2. End that process in the test's own cleanup, on a pass and on a failed assertion.
3. Keep every assertion of the test as it is.

**Tests:** the named test stays under 1 s. Nextest no longer reports it as leaky.

**Files:**
- `crates/cargo-tile/tests/cli_lifecycle.rs`

**Workers:** 1 writer (`fix`).

**Acceptance gate:**
- `verify.sh lint cargo-tile` and `verify.sh test cargo-tile --mac` green; nextest on the Mac reports no leak for this test.