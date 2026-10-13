# tile-fixes

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-tile draws a moving cell on the painted ground, keeps every label and readout whole or marked, follows the system's light or dark setting, shows the summary alone when the window is too small for cells, sizes the summary to what it asks, and nextest starts the slow tests first.

> **Production: remove-running-section** — unit `tile-fixes-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

> Renumbered 2026-10-12: the slow-test shortening was split out of the old phase 19 as phase 17; the old phases 17, 18, 19, 20 are now 18, 19, 20, 21.

> Renumbered 2026-10-13: phase 19's design-check follow-up was inserted as phase 20 (hard landing); the old phases 20 and 21 are now 21 and 22.

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
  - `crates/tui_pane/src/tiles/draw.rs` — `draw_tile_grid` (144), `draw_placements` (168), `has_visible_body_row` (267), `draw_cell` (314), `draw_rows_readout` (405), `draw_rows_readout_after_foot` (411), `rows_readout_line` (432), `draw_rows_readout_line` (465), `draw_summary_foot` (473), `content_area` (499), `readout_area` (517), tests from 546 (`StubCells` 560, `draw_motion_fixture` 644, `settled_grid` 932, `draw_tile_cell_draws_only_the_readout_on_its_foot_row` 1443).
  - `crates/tui_pane/src/tiles/grid.rs` — `TileDrawing` (167), `CellTransition` (240), `BandPieceMotion` (271), `TileGrid` (396), `sync` (622), `target` (822), `queue_with_depth` (850), `cycle_focus` (1057), `cell_at` (1108), `progress` (1125), `placements` (1138), `drawing` (1144), `drawing_at` (1149), `Grid::new` (1402), `columns` (1624), `shares` (1702), `fits` (1777), `summary_share` (1790), `summary_depth` (1810), `shared_run` (1884), `moving_cell` (1991), `wrapping_cell` (2081), `column_bands` (2120), `column_endpoint` (2164), `place_band_pieces` (2246), `band_piece_dividers` (2296), `piece_frame` (2360), `lerp_rect` (2436), tests from 2484.
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
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check cargo-tile`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile`
- **Style:** run-end /clippy style-only auto-proceed
- **Invariants:**
  - A value on screen is whole, or shortened with a `…` that marks the cut, or absent. Nothing is cut without a mark (showrunner's gate for this unit: nothing clipped mid-value).
  - A command cell is never narrower than 40 cells in cargo-tile. A window that cannot hold the grid at that width shows the summary alone, filling the window (user, 2026-10-08: cells are useless when too small).
  - Every literal the code names lives in its crate's `constants.rs` (existing crate convention).
  - No test takes a second. Render tests are pure and run on buffers; none waits on the clock (user rule, memory `no-multi-second-tests`).
  - A `tui_pane` change reaches `cargo-handler` and `cargo-port`; a gate that touches `tui_pane` also runs `verify.sh test` for both (production doc, Merge tests). No existing `tui_pane` public signature changes, and `cargo-handler`'s cell sizes do not change (author's calls). The approved exceptions (the user, from the Phase 12 review) are the public signatures Phase 18 names — toast timing, toast action, toast focus and tracked-item timing — and `TileCells::group_title`, which Phase 21 changes. Phase 21's other public items are additions only: `draw_tile_grid` keeps its signature, and a retained boundary that answers `Option`, such as `cell_at`, converts from a named internal answer at that boundary alone.
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

### Phase 20 — Each reading in one place, and every mark where its cut is  · status: todo

#### Work Order

**Goal:** Each command's progress reading is drawn once, where its row is; every hidden ancestry level, cut path and dropped account tag is marked the same way at the place it was cut; and the Short command column and the `state` column each draw one form. Follow-up to Phase 19 under the production's hard-landing rule: phase 19's design check on its repaired build (2026-10-13, shots `ph19b`) found these, none a regression of that phase, so they moved here.

**Spec:**

1. **A reading is drawn once, beside its own row** (shots deep126x80, c64, c48x120, cdrv126x60, w200, toast64; c200, cdeep126x120; w48, w40). Three defects of one rule. First, a summary heading over several commands carries one reading and no row says whose it is: `~/rust/tool-based-ui-trunk testing … 79.5%` over nextest 1058771 and check 1221119; cdrv126x60 `building 99.8%` over 1041926 and 1057863, where the reading is 1057863's. Second, the same reading is drawn twice: the summary heading or row repeats what the command's own cell heading shows (c200 trunk `87.4%`, lfo `95.9%`; cdeep126x120 obsidian rows `29%`, `1%`, `43%`, `1%`). Third, a one-command directory's reading lives only in its heading, so a heading too narrow for the bar drops it while the row's `state` cell stands empty (w48 and w40 switch-alone, `93.0%` at w64). Rule: in the summary a command's reading sits in its row's `state` cell, never in a directory heading; a command cell's heading carries its own command's reading as today. A command that has a cell on screen still shows its reading in the summary row, so the summary reads alone. This replaces Phase 19 item 1's rule that a heading carries a gauge when one command in its group has a reading. Tests: a summary group with two commands of which one has a reading draws the reading in that row's `state` cell and none in the heading; a one-command group at 40 columns draws its reading in the row's `state` cell; Phase 19's `two_commands_in_one_directory_show_their_own_progress` and `a_heading_does_not_repeat_its_cells_progress` change only where the heading's gauge moves to the row, each named in the summary with old and new text.
2. **Readings line up** (w48, w64, toast200, cdeep126x120, cn16x120, cn22x120). The `state` column's `%` signs fall on different columns (w48 `66.1%` and `35%` share a left edge while ` 7%` is right-aligned), and the `state` title starts one column left of its values. Rule: every reading in the `state` column is right-aligned so the `%` signs share one column, and the title aligns with the values' right edge, as `cpu` and `mem` do. Test: a summary holding `66.1%`, `35%` and `7%` draws their `%` in one buffer column under a title that ends on that column.
3. **A hidden ancestry level is always marked** (c40x120, c48x120, cdeep126x120, cdrv126x60; c200, c40, cdeep126x80, cdeeplong126x120). Short view draws a one-row ancestry `982432 claude …` whether it hides bash and zsh below claude or none (`1661018 codex …`, a direct parent), and the trailing `…` is the argument mark, so hidden levels below the kept one carry no mark. A cell that hides its whole ancestry marks it nowhere; only the red foot count says so, and in Short view the `parent` cell is empty. Rule (approved by the showrunner, 2026-10-13, with rules 1 to 6): Short view's one-row ancestry drops the argument mark (Short view always drops arguments), and when levels stand hidden between the kept level and the table's row, the row ends in ` …`, so a trailing `…` means hidden levels only. A cell whose ancestry is hidden entirely keeps its row's parent pid in the `parent` column. Tests: Short-view ancestry over a direct parent draws no `…`; over a chain with two hidden levels below the kept one it ends in ` …`; a cell with no ancestry row draws its row's parent pid. Condition (showrunner, 2026-10-13): no Short-view row shows `…` with two meanings. Test: a Short-view cell whose command name dropped words, drawn beside an ancestry row with hidden levels, reads each `…` from the buffer as what it marks: the command row's as dropped words, the ancestry row's as hidden levels. A cut executable name is the third case: in Short view a long runner basename clipped to one row (`render.rs` ~1399, `AncestryWrapping::SingleRow` through `mark_cut_line`) also ends in `…`, even as a direct parent with nothing hidden. Rule (unit director's call, 2026-10-13, sent to the showrunner): a cut basename ends with `…` directly after its last drawn character, the general cut form; hidden levels are marked by ` …` after a space at the row's end; named ancestry-visibility states, such as `AncestryVisibility::{Whole, LevelsHidden, CutAtWalkLimit}`, carry which mark applies, so no bare flag or text test decides it. Test: `each_short_ancestry_mark_reads_as_what_it_marks`, a buffer test combining a long basename cut, hidden lower levels and a walk-limit cut, reads each mark as what it marks.
4. **A cut ancestry path ends at its cut** (cdeeplong126x120: `… 924321 bash /tmp/claude-1000/-home-natepiano-rust-widget-enhancements/29a762ed-87d9-4b88-aa53-` stops at a wrap point with its `…` 25 columns to the right at the border). Rule: a Long ancestry level cut by its row budget fills its last row with as much of its text as fits and puts the `…` directly after the last character drawn, as a cut command row does. Test: a Long ancestry level whose wrapped text runs past its last allowed row draws its mark in the cell directly after its last character, with no blank cells between.
5. **One form for a Short command that overflows its column** (w40, toast40: `[p19b] cargo build…` glues the argument mark to the subcommand, so the subcommand reads as cut; `[release] cargo` / `build …` wraps to two rows while every other Short row is one). Rule: in Short view the subcommand stays whole and the argument mark keeps its space (` …`); a name that does not fit its column on one row takes cells from the `state` column's spare width before it wraps, and when it still wraps it breaks between words, the same way in every row. How many cells a name borrows from the `state` column's spare width is decided once per frame grid, never per table, so every table that keeps the slot draws the columns after it at one offset. Tests: at 40 columns `[p19b] cargo build …` draws whole on one row; `[release] cargo build …` draws on one row when the `state` column has spare width, and wraps at a word boundary when it has none. Also cnd24x40 and cn30x120 (shots `ph19d`): `1725285 cargo men…` cuts the subcommand word and `1726554 cargo fmt…` glues the mark to it while the same table wraps `cargo` / `clippy …` whole and keeps two spare rows; the rule above covers them.
6. **One account form per table** (w48, w40, toast40, c40, c40x120, c48x120, cdeep126x120: `~/.cache/trace-targets/switch-alone` beside `[natepiano] ~/rust/tool-based-ui-trunk` in one table; cn22x120 `…/obsidian_knife` beside `…hana_catalyst-lfo`). Rule: when any heading in a table drops its account tag, every heading of that account in the table drops it; a heading of another account keeps its tag, so a mixed table still names each other account. A dropped path prefix is always written `…/` when the last segment is whole. This replaces Phase 19 item 7's per-heading choice. Tests: a 40-column summary with two directories of one account, one of which fits with its tag, draws both without it; a table with a second account keeps that account's tag; a cut prefix before a whole last segment reads `…/<segment>`. The account-wide drop never removes a heading's only identification: an unreadable directory's heading keeps its account-only form (`[natepiano]`, `an_unreadable_directory_heading_keeps_only_its_account`), and the drop applies to headings that still name a path. Test: `an_unreadable_heading_keeps_its_account_beside_a_readable_one_that_drops_it`, a table holding a readable and an unreadable directory of one account, where the readable heading drops its tag, draws the unreadable heading as `[natepiano]`.

7. **A hidden driver's row is named by the process it dispatched** (cdrv126x60, cdrvlong126x60, shots `ph19c`). The driver's ancestry row reads `3573610 cargo p19cdrv`, the `cargo` shim, and the `parent` cell of `3573614 cargo nextest run …` reads `3573610`; in `ps`, 3573614's parent is 3573612 (`cargo-p19cdrv p19cdrv`, the dispatched driver). Nextest rows already keep the dispatched process's pid (3528933 `cargo-nextest`, its shims not drawn). Rule: a driver row, like a nextest row, carries the pid of the process cargo dispatched, and its child's `parent` cell names that pid. Test: a census of a shim that dispatches a driver which runs `cargo nextest run` draws the driver row with the dispatched pid and the child's `parent` cell equal to it. The same rows draw the driver's parent pid (`1661445` on row 1661464, cdrv126x60 #8c96ff, cdrvlong126x60 #ff6ecf) in a family colour though no row in the cell carries that pid; a parent pid with no row in its table is drawn in plain ink, as every other root row's is. Test: that driver row's parent cell reads the plain ink.
8. **The hidden-rows mark sits at the end of its row** (c200, c64, c48, c40, w40_cut; long64, long40, clong64, cutfoot). Short view draws the hidden-rows `…` at a frame-wide column one cell past the frame's longest command, so it floats mid-row (`cargo nextest run …     …`), or lands one cell after an argument mark (w40_cut `cargo tw … …`, two marks of different meaning side by side). Rule: the hidden-rows mark is drawn in one place in both views, at the row's right end as Long view draws it, never at a column reserved across the frame. Test: a cut Short summary whose last drawn command is short draws the hidden-rows mark at the row's right end, with no `…` beside the argument mark.
9. **The toast never covers the summary's foot** (toast64, toast40, toast200). While the toast is up, its band runs through the summary's foot row (toast64 and toast40 lose `mem …` and the count; toast200 covers the count at the foot's right end) while rows above it stand empty. Two places decide what the toast covers: the toast area cargo-tile passes (`crates/cargo-tile/src/render.rs` ~274, `tui_pane::render_toasts(frame, &mut app.framework, tui_pane::frame_inner(body))`), and `clear_toast_surroundings` in `crates/tui_pane/src/toasts/render/card.rs` (~150), which clears `TOAST_ROW_GAP` rows below the card (~157) and so erases a foot row the card itself does not cover. Rule: the toast is placed, and its surroundings cleared, so the foot row and its count stay drawn. Test: `a_toast_leaves_the_summary_foot_drawn`, a frame with the toast up, reads the card's bounds, the region cleared around it, and the summary's foot count and `mem` total in the buffer, so a placement that clears the count fails.
10. **Separators come before a column mate's spare row** (cdrv126x60 rows 3–4 and 23). The summary gives up one separator (`cargo nextest run …` directly above the `/tmp/…/light_tip_tree` heading) while the cell below it in the same column (repair-alone, `content rows: 6  r/c: 8/124`) keeps a blank row above its foot. Rule: a column's spare row goes to the summary's separators before any column mate keeps one. Test: a column holding a summary short one separator row and a cell with one spare row draws every separator and no blank row above the cell's foot.
11. **No untitled gap in a cell's table** (cdrvlong126x60, shots `ph19d`). In view cells, one cell titles `state` for a blocked `cargo check`, and the six cells beside it keep the frame-wide `state` slot as an 8-cell blank run with no title (`mem           command`). The slot stays in the frame grid (Phase 14) while each table labels it only when its own rows hold a value (Phase 18). Rule: a slot that stands in a table's grid is titled in that table, so a frame whose grid holds `state` titles it in every table that draws the slot; the grid itself is unchanged. Also clong64 and cutfoot (shots `ph19e`): with a reading elsewhere in the frame, every Long cell keeps the empty slot and drops `dur` (clong64) or `start` (cutfoot) to make room for it; a titled empty slot must not cost a column that holds values, so where the slot would push out a valued column, the slot leaves that table (`CellTableLayout::for_table` removes it only for the summary today), and the columns after it in that table shift left by the slot's width. This is the one exception to the frame grid's shared offsets (Constraints). Tests: two cells in one frame, one with a `state` reading and one without, both draw the `state` title at the same buffer column; a 64-column Long cell without a reading beside one with a reading keeps `dur`; `an_empty_state_slot_leaves_only_a_narrow_table`, two narrow cells in one frame, one with a `state` reading and one without, reads back every surviving header's offset: equal in both cells before `state`, and in the cell without a reading shifted left by the slot's width after it.
12. **No placeholder for a missing reading** (toast64 row 1457506, cutfoot row 1458672, shots `ph19d`). A process with no `cpu` reading yet draws `--` (`UNAVAILABLE_MEASUREMENT`) in its `cpu` cell: neither a whole value, a marked cut, nor absent. Rule: a measurement that has no reading draws a blank cell until it has one. Test: a row whose cpu is unavailable draws only spaces in its `cpu` cell.
13. **A wrapped line breaks where a word fits whole** (long40, cdeeplong126x120). `| test(rear_jack_` / `pixels) |` splits a 22-cell word in a 22-cell column; row 1407016's path breaks only at `-`, so a middle line uses 48 of about 84 cells while a `/` break was available. `crates/tui_pane/src/wrap.rs` already moves a word that fits a fresh line there whole (`push`, ~108), and `WRAP_BREAK_AFTER` already holds `/`; two defects remain. First, `push_breakable` (~172) takes the last hyphen in the line before any `WRAP_BREAK_AFTER` break, so a `/` after that hyphen never wins. Second, the 22-cell case comes through `process_row` in `crates/cargo-tile/src/render.rs` (~3801), which wraps the command at its column width less `continuation_clearance`, so a word as wide as the column splits. Rule: a word that fits a whole line of its column moves to the next line whole, and `/` is a break point as `-` and `_` are, so a `/` after the line's last `-` takes the break. Tests: `a_column_wide_word_wraps_whole_through_process_row` draws a 22-cell word through `process_row` in a 22-cell command column on its own line whole; `a_slash_after_the_last_hyphen_takes_the_break` wraps a long path through `wrap.rs` at `/` when that fills the line further than the last `-`.
14. **A row reads the same in the summary and in its cell when both draw the same rows** (cdrv126x60, cdrvlong126x60, shots `ph19e`). The row under a hidden driver, `3801638 cargo nextest run`, reads its subtree in the summary (`438%  1.0G`) and its own process in its cell (`0%  0.0G`), while every other parent row reads the same in both. `summary_rows` gives a promoted row the subtree total; the cell keeps the row's own reading. This was so before Phase 19. Rule: a pid reads the same value in the summary and in its cell when both tables draw the same contributors. Where the summary draws a row without its children (its one row per command), that row reads its whole subtree, and its cell, which draws the children, reads by item 15's scope, so the two differ by exactly the drawn children's share. Tests: `a_pid_reads_one_value_where_both_tables_draw_its_rows`, a frame where the summary and the cell draw the same rows, reads equal `cpu` and `mem` for the pid in both; `a_summary_row_reads_its_subtree_while_its_cell_reads_its_own_share`, a parent at 10% with a drawn child at 90%, reads `100%` in the summary, where the child has no row, and `10%` in its cell.
15. **A parent above one child does not repeat its child's numbers** (cdeep126x80, cdeep126x120, cdeeplong126x120, cdrv126x60, cdrvlong126x60, cnd126x40, shots `ph19e`). A parent row counts its child's subtree, so a parent over one child draws the child's `cpu`, `mem` and `sccache×N` again on the next row (`3784749` / `3784767` `2%  0.3G` `sccache×6` both). The compiler count is drawn twice in one table, and two equal readings read as a copy or as twice the load. This was so before Phase 19. Rule (showrunner's call, 2026-10-13): a row reads its own process plus the descendants that have no row of their own, so no reading is counted twice; a row whose children are not drawn (a cut, or the summary's one row per command) reads its whole subtree; the compiler count stands only on the row whose unrowed descendants are the compilers. Readings come from numeric contributions, never from formatted strings. Today `crates/cargo-tile/src/census/scan.rs` (~2117) overwrites the group lead's `cpu`, `memory` and `compiler` with the group's totals, and `CargoProcess::cpu` holds a formatted percentage (`Measurement<String>` from `cpu_label`), so the drawing cannot split a reading after a row is cut. The census keeps each process's own numeric cpu, memory and compiler count under one semantic type, `InvocationResourceUse`, and the drawing derives each row's reading from the final table plan: its own process plus the descendants that have no drawn row, summed from `InvocationResourceUse` values and formatted once, never by subtracting rounded strings. A sum with an unavailable contributor reads unavailable, as the census's group total does today. Tests: a parent with one drawn child draws no `cpu`, `mem` or compiler count equal to its child's unless its own process holds them; `a_row_reading_sums_its_known_contributions` (known totals); `an_unavailable_contributor_leaves_the_row_reading_unavailable`; `nested_unrowed_descendants_add_to_their_nearest_drawn_row`; `a_cut_table_moves_hidden_rows_use_into_their_drawn_ancestor`.


**Files:**
- `crates/cargo-tile/src/render.rs` — `heading_gauge`, the summary rows' `state` cells and their alignment, `ancestry_levels`, `ancestry_lines`, `mark_cut_line`, `PathGroup::fitted_heading`, the command column fit, `process_row`'s wrap width, each row's reading derived from the final table plan, the toast area (~274), and their render tests.
- `crates/cargo-tile/src/constants.rs` — any literal the rules name.
- `crates/cargo-tile/src/census/scan.rs`, `crates/cargo-tile/src/census/process_identity.rs` — the pid a driver row carries (item 7) and each process's numeric `InvocationResourceUse` (item 15), and their tests.
- `crates/tui_pane/src/toasts/render/layout.rs`, `crates/tui_pane/src/toasts/render/card.rs` — where the toast is placed against a tile's foot and the region `clear_toast_surroundings` clears (item 9), with its CHANGELOG line in `crates/tui_pane/CHANGELOG.md`; the toast area cargo-tile passes is in `render.rs` (~274).
- `crates/tui_pane/src/wrap.rs` — the break points of item 13 (any edit also runs `test cargo-tile`).
- `crates/cargo-tile/CHANGELOG.md` — one Changed line per rule the user sees.

**Seats:** 2 writers — the census half of items 7 and 15 (the driver's dispatched pid and `InvocationResourceUse`) is disjoint from the drawing; everything drawn, `tui_pane` included, has one owner so the table plan, the readings derived from it and the ancestry fit stay in one hand.
- `impl` — drawing, shared table measurement, wrapping, toast placement, constants, changelogs and render tests: `render.rs`, `constants.rs`, `wrap.rs`, `toasts/render/layout.rs`, `toasts/render/card.rs` and both CHANGELOGs; items 1 to 15 except the census half of items 7 and 15.
- `test` — `census/scan.rs`, `census/process_identity.rs` and their tests: the driver's dispatched pid (item 7) and `InvocationResourceUse` (item 15); publishes those APIs on the board before `impl` consumes them.

**Constraints from prior phases:** Phase 19's rules hold except where items 1 and 6 replace its items 1 and 7: one ancestry form with the mark inline in front of its pid at its depth; the `state` column stands only while a drawn row holds a reading (Phase 19 repair round 4); separator rows the summary gives up are not hidden content; a command names its profile when it is not the default. Phase 19's profile tests stay green unchanged: `cargo_profiles_survive_a_cut_command_in_both_views` and `long_rows_keep_profile_arguments_beside_their_label`. The frame grid's columns keep shared offsets in every table (Phase 14), except that an empty `state` slot leaves a table where keeping it would push out a column holding values (item 11); the columns after it in that table shift left by the slot's width. Phase 14's optional-column rule holds; `cpu` and `mem` keep one width for the session. Phase 9: tables are planned before drawing, and a cut command fills its column before its mark. No new `#[allow]` or `#[expect]`; no test takes a second.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- Unit director's captures of the shot set Phase 19 used (summary, Long view, toast, deep, cells, driver and narrow shots): no directory heading in the summary carries a reading; no reading is drawn twice in one table; every `%` in a `state` column falls on one column; no Short ancestry over a direct parent ends in the hidden-levels mark ` …`; no table mixes a tagged and an untagged heading of one account; no `…` stands apart from the text it cuts.
- A fresh helper's design check of those shots: pass.

### Phase 21 — Every cell moves toward one target layout  · status: todo

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

9. **A summary that can give a row gives it on the arrival frame, even while a piece is leaving** (the showrunner's placement, 2026-10-12, from phase 15's live frame check, which read worse than phase 14 on this alone). Seen: a third project arrives while the summary stands eight rows high with no spare row and a leaving piece two rows high stands below it; for two frames (36 ms) the summary draws its header, two projects, one blank row and its foot, with the third project hidden behind the cut, and only then gives the row. Rule, written as behaviour so it holds for whatever motion model this phase lands: on every frame, a summary that hides a project behind its cut draws no blank row above its foot; when an arrival leaves one of its rows unused, its height drops on that same frame, whatever else in its column is moving. Cause in today's code, for reference only: `snapped_band_piece_dividers` in `crates/tui_pane/src/tiles/grid.rs`, on `FilledBandPath::NoFilledPath`, returns the heights held before whenever their dividers reach the band floor, without rechecking that a resident now leaves a gap; a leaving piece two rows high makes the joint search fail (`piece_span_bounds`'s leaving guard and `snapped_piece_fit`'s minimum-height rejection). That file is the same in phases 14 and 15, so phase 15 did not cause it. Evidence: `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/live15_reply.txt`, `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/probe12/out/results_p15.txt` (sections `P15 gates` and `P15 control`), and the kept captures `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/probe12/keepcap_c_p15b/c_cells7_r1.json` (key8 `-`, +0.317 s) against `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/probe12/keepcap_c_p14ctl/c_cells7_r2.json` (key8 `-`, +0.223 s). Test: `a_summary_gives_its_unused_row_on_the_arrival_frame_while_a_piece_leaves`, through the real draw path, reading the drawn buffer back on every frame.

10. **A job cell that changes columns lays its content out once** (add-on, answered by the user via the showrunner, 2026-10-12: covered by the rewrite). A job cell's heading and ancestry are its name, and today its two pieces lay content out separately (`draw_placements`, `TileCells::draw`; cargo-tile's `Cells`), so a row shows twice or drops out while the cell changes columns, a closing column included. Evidence: `P/ph3s/shots/ph3s_on_12_close_motion.png`, `ph3s_off_09_close_later.png`, capture `P/ph3s/off/cap.json` key 13 at 1.2 s. On the new model a cell is one rect, so this holds by construction; a render test with a job cell crossing columns in both directions shows each content row at most once per frame.
11. **A `-` pressed during a motion starts at once** (add-on, answered as item 10). Today a queued key waits in `pending` until the preceding transition's 720 ms timer ends, 0.64 to 0.93 s late with nothing changing on screen (`P/ph2d/off/ph2d_off_08_close_motion.png`). On the new model a key retargets every cell on the frame it arrives; a test presses `-` mid-motion and sees every cell's target change, and the first moved frame, within one frame.
12. **A key that lands while a command's motion runs joins that motion** (from phase 14's live check, by the showrunner's landing call, 2026-10-12). Evidence, the controlled workload (200x50, cells view) on phase 14's build after its second repair round: a `+` that lands 7 to 112 ms after a command claims a cell waits 0.96 to 1.08 s for its slot, because the key queues behind the command's motion. In that wait the summary takes 4 rows and gives them back when the slot enters (second round, sixth `+`: height 13 to 17 at +0.451 s, 17 to 13 at +1.077 s, 13 to 11 at +1.438 s), and a command cell takes one row and gives it back (third `+`, both rounds: 5 to 6 at +0.281 s, 6 to 5 at +1.421 s). Rule (showrunner's call, 2026-10-12): a key that lands while a command's motion runs joins that motion, and no piece takes a row it gives back. This holds for a command whose cell opening has not yet entered, one that has claimed an existing cell, and one whose cell has entered; on the new model the key retargets every cell on its frame. Tests, on the render path: `a_key_landing_after_a_command_claims_a_cell_joins_its_motion`, `no_piece_takes_a_row_it_gives_back_when_a_queued_key_enters` (no piece's height rises and then falls back within the key). Gate: the unit director's capture reads 0 reversal flags and one motion on every key that lands within 0.15 s after a command arrives or claims a cell.
13. **A cell's group title is a named title** (the title is drawn by the painter this phase replaces). `TileCells::group_title(&Id) -> Option<Span>` becomes a semantic title such as `TileGroupTitle::{Untitled, Titled(..)}`, with its match in the drawing, its exports from `tiles/mod.rs` and `lib.rs`, and every implementor: cargo-handler (`crates/cargo-handler/src/render.rs`, `group_title`) and `draw.rs`'s test `StubCells`. cargo-tile keeps the provided default, which answers `Untitled`. What the screen shows does not change.
14. **A cell that changes column or order position closes out of its old spot, then opens in its new one** (showrunner's ruling, 2026-10-12). Over the first half of its travel the cell closes out of its old spot; over the second half it opens in its new spot. It is one rect at every frame, never overlapping or crossing another cell, and it keeps its content, title and focus throughout. Commands that end together close together, not one cell at a time. A straight slide drawn over its neighbours is not allowed. `a_cell_changing_column_is_drawn_in_both`, `commands_ending_together_close_one_cell_at_a_time` and the reorder tests (`a_cell_the_order_overtakes_closes_and_comes_back_in_behind`, `the_order_changing_cascades_down_the_rest_of_the_grid`, `no_step_of_a_reorder_draws_two_cells_crossing`, `a_reorder_leaves_the_cell_count_where_it_found_it`, `focus_waits_for_a_cell_that_closed_to_let_the_order_through`) are ported to this rule, each keeping its titles, content identity, focus, final order and non-crossing assertions.
15. **Filled heights win over the three-row step** (showrunner's ruling, 2026-10-12). When the nearest height a resident's content fills is more than three rows from its displayed height, the cell moves straight to that height as a named transfer, as a content shrink does under item 5, so no blank row ever shows. Item 4's limit excepts this transfer. `a_height_search_reaches_the_nearest_valid_height_each_way` keeps its 19 and 27 heights.
16. **An entering cell keeps its displayed height while its band has room** (showrunner's ruling, 2026-10-12). A retarget that asks an entering cell for less height than it displays leaves the cell at its displayed height while its band can hold it. When the band cannot fit it, the cell moves toward its new target at the resident rate, and its admission clock continues; it is never restarted.

17. **Two motion findings from phase 19's re-shoot** (the showrunner's routing, 2026-10-13; shots `ph19b`, captures `cap7c` and `cap6`). First, each key's last moving frame still differs from the settled frame after it: cap7c 7/14 and 10/14 keys, cap6 6/14 and 5/14; the phase 18 build reads 10/14 and 10/14, and 4/14 and 4/14, so it predates phase 19. The pattern: resident cells one row taller at the last moving frame than settled, cells below one row up, the entering cell one row taller settled. Second, one column widened by one cell during motion: in cap6 `w200_r1`, the cell's `parent` column widened at 4.678 s and `start`, `dur`, `cpu`, `mem` and `command` each moved one cell right. Today `FrameColumnGrids::for_frame` (`crates/cargo-tile/src/render.rs` ~2500) rebuilds the parent widths on every frame. Rule: the last moving frame equals the settled frame (this phase's one-target-layout design), and the column widths a motion draws are one named policy, `MotionColumnWidths`, held for the motion's lifetime: computed at the motion's start, retargeted only at a key or scan retarget boundary, widened (never narrowed) while the motion runs when a longer reading or pid arrives, and narrowed only on the settled frame. It keeps phase 19's session-retained `cpu`, `mem` and compiler widths, and it reads the motion's start, retarget and settled states from the prepared frame. Tests: a motion whose final frame is compared cell for cell with the settled frame after it; `a_longer_parent_pid_widens_its_column_once_during_a_motion`, a cell whose rows gain a longer `parent` pid during a motion, draws every column at one offset in every frame before the pid arrives and at one offset in every frame after it; `motion_column_widths_narrow_only_on_the_settled_frame`, a motion whose rows lose their longest pid, keeps every offset until the settled frame.

18. **Columns fit the rows the final allocation draws** (phase 19's routing, 2026-10-13). `possible_frame_rows` (`crates/cargo-tile/src/render.rs` ~2574) filters rows only by the frame-height ceiling, so a long row the final allocation leaves out can still remove `cpu` and `mem` from every table. Rule: preparation fits the frame's columns to the rows the final allocation draws, with a bounded fit and the shared measurement (`Cells::row_use`, `FrameColumnGrids`), so no row the screen does not show takes a column from one it does. Test: `an_allocated_out_long_row_keeps_cpu_and_mem`, a render test where the final allocation leaves a long row out, draws `cpu` and `mem` in every table.

**Design** (approved by the showrunner, 2026-10-12). The rewrite keeps one target layout and one displayed rect per cell. Neighbouring cells share separators, conserve their column's rows and hand useful content rows over in one frame, so the grid advances by one joint step rather than by independent rect interpolation.

*Model.* The public `TileGrid`, `TileDemand`, `TileDemands`, `TileContent` and `TilePlacement` interfaces stay. These semantic types replace the internal motion implementation:
- `CellIdentity<Id>::{Summary, Command(Id), Vacant(VacantCellId)}` names a physical cell apart from its displayed number and content. A command claiming a vacant cell keeps that cell's identity, rect and admission timing.
- `DisplayedCell<Id>` owns its identity, current content, one displayed rect, presence and travel clock. It owns no source rect, endpoint copy, previous-height vector or queued destination.
- `CellPresence::{Resident, Entering(AdmissionTiming), Leaving(DepartureTiming)}` selects the motion rule. An existing entry stays entering across a retarget; claiming its cell does not create another entry.
- `CellTravel::{Settled, Travelling(TravelTiming)}` says whether geometry needs another frame. `TravelTiming` holds elapsed and remaining duration, not a source layout.
- `TargetLayout<Id>` holds the settled cell rects, column bands, summary depth and span, focus navigation lanes and the resolved display mode.
- `CellTarget::{Present(Rect), Withdrawn(ExitEdge)}` tells an ordinary destination from removal. A withdrawn cell keeps its displayed rect until its slide has finished.
- `CellRowUse` holds painted rows, retained rows and `ContentFit::{Filled, Whole, HidesContent}`. Painted rows decide whether a receiving allocation draws more; retained rows protect donors.
- `RowTransfer::{None, ImmediateContentRelease { donors, recipients }}` records the one event in which every participating cell may exceed the three-row limit (items 5 and 15). The update produces it; the painter never infers it.
- `RetainedCellAllocation` keeps today's stepped-demand stickiness as an explicit input to layout, never a second geometry authority.
- `GridCellHit::{Outside, Summary, Cell(..)}` and `RemovalCandidate::{NoneAvailable, VacantCell(..)}` replace the internal optional answers; the public `cell_at -> Option<usize>` converts at its boundary alone.

*Modules.* Target allocation lives in `tiles/target_layout.rs`, motion in `tiles/motion.rs`, and identity and state ownership plus the existing public methods in `tiles/grid.rs`. Timing literals stay in `tiles/constants.rs`; the ripple settings leave `tiles/settings.rs`. `tiles/mod.rs` and `lib.rs` own the additive preparation API.

*Target layout.* `target_layout(inputs, content) -> TargetLayout` is pure in its observable result. Its inputs are the area, growth, floor and view settings, the reconciled cell identities and order, focus, the current content revision, the retained demand and allocation policy, and immutable content measurements. It reuses the existing `columns`, `fits`, summary depth and span rules, apportioning, summary priority and focus priority. It evaluates content at the candidate destination widths before it publishes a target, so no provisional depth is published and repaired on the first draw; the width and depth search is bounded and deterministic, with an explicit tie order. A fresh target is computed whenever a scan changes relevant content, a key changes cells or focus allocation, settings change, or the terminal resizes. An unchanged observation may reuse its result, but never restarts travel because another frame occurred.

*Step.* `step(displayed, target, dt) -> DisplayedLayout` takes the whole displayed grid. It first proposes each cell's advance, then resolves shared column boundaries and useful row transfers jointly. It uses integer fixed-point coordinates and an injected duration, and reads no `Instant` inside layout or stepping. For unchanged topology and unrestricted geometry, the residual easing fraction `(ease(elapsed + dt) - ease(elapsed)) / (1 - ease(elapsed))` advances the current rect toward its target without a stored source rect; it clamps at the remaining duration and lands exactly on the target. Stable cell order decides rounding and every allocation tie.

*Joint projection.* The step conserves each band's rows, shares each separator, respects resident floors, prevents ordinary resident height reversals toward a fixed target, and selects useful filled allocations from the current content measurements. A small leaving cell does not invalidate the resident allocation: its timed allocation is accounted for apart from resident content-fit eligibility, so today's `FilledBandPath::NoFilledPath` fallback that keeps an obsolete gap goes.

*Retarget.* A retarget replaces the whole target at once and starts from the stored displayed rects. It never rebuilds positions from a previous endpoint or advances an old queued step. Entering and leaving clocks survive content-only retargets; a newly admitted cell gets its own clock. A new key updates every target on its frame, with resident travel timing assigned to that key; unchanged frame observations do not extend its deadline.

*Opening and withdrawal.* A new opening starts at its named border-only opening allocation and gets 720 ms from its own admission, whatever another cell's elapsed motion. A leaving cell releases rows along its slide and is erased only at its terminal invisible allocation. Item 16 sets what an entering cell does when a retarget asks it for less than it displays.

*Preparation and painting.* A preparation operation observes `TileCells`, resolves the target, gathers the existing shared row measurements, steps once, and publishes an immutable `PreparedTileFrame`. The painter reads that frame and calls the content provider once per visible cell. `draw_tile_grid` keeps its public signature and performs no layout mutation; cargo-tile and cargo-handler prepare before calling it. Hit-testing and navigation read the same published geometry. cargo-tile keeps its immutable `FrameColumnGrids` and the shared `Cells::row_use` measurement in `render.rs`: measuring at candidate sizes is distinct from drawing, and each displayed cell gets one final content layout and one paint. Phase 16's `CellReadoutFade`, focused ground selection, summary-foot fading and whole-or-absent readout candidates stay.

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

**Test map.** This covers all 158 `grid.rs` tests and 74 `draw.rs` tests. *Keep* means the test body and its assertion stay unchanged, with any pure helper they need. *Port* means every visible assertion stays while fixtures that write `held`, `Transition`, source geometry, queued states or height vectors become displayed-cell and target fixtures. Arbitrary `drawing_at(raw)` samples become deterministic update-and-step samples (independent fixture states where the samples are deliberately unordered); queue completion becomes stepping to settled geometry; and a test inspects the immutable prepared frame instead of rebuilding an endpoint during drawing.

`grid.rs` — keep:

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

`grid.rs` — port:

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
a_demand_change_in_one_frame_decides_the_view_from_that_frames_rows
changed_row_answers_recommit_an_unchanged_settled_view
a_one_row_change_in_the_summarys_ask_does_not_flip_the_view
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
```

Port notes for `grid.rs`:
- Floor, summary depth and span, and sticky-allocation tests read the pure target and the explicit retained policy.
- Admission, claim and focus tests inspect stable physical identities and the prepared displayed layout.
- Height tests read current rects after `step`; `HeightSnap` and divider-search fixtures become measured content-fit fixtures, under item 15 for sparse heights.
- The departure-recording test seeds the displayed rect directly; no settled-source snapshot is needed.
- Probe tests assert upper bounds: 29 moving calls and the existing settled measurement budget, with uniqueness kept and `tick() == false` when settled. Exact equality would reject a cheaper implementation.
- "Queues one step" and "queues nothing" tests keep their one-motion and no-extra-motion guarantees without inspecting a queue.
- Changed-column and sequential reorder or departure tests port to item 14. Resize tests keep containment and the rule that stale geometry never returns, without assuming every resize ends motion.

`grid.rs` — retire: `every_queued_state_keeps_surviving_cells_in_an_endpoint_column`. It enumerates `pending` arrangements and checks their endpoint-column assignment; those intermediate queued arrangements no longer exist. Its visible containment protection stays in the port of `adding_the_tenth_live_cell_never_draws_a_survivor_in_a_third_column` and in the live changing-width checks.

`draw.rs` — keep:

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

`draw.rs` — port:

```text
a_cell_lends_no_row_it_must_take_back_within_one_key
a_toast_width_summary_never_draws_compact_with_room_above_its_foot
a_turn_that_grows_content_and_height_draws_no_blank_row
a_summary_hiding_rows_beside_cells_shows_alone
a_width_responsive_summary_uses_each_views_own_demand
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
```

Port notes for `draw.rs`: hand-built `TilePiece` fixtures become one-cell prepared-frame fixtures, and public-grid tests prepare before painting. Buffer text, glyph, ground, title-count, fade, useful-row and floor assertions stay. Clock and source-cache assertions become admission and deadline continuity and latest-target assertions. `a_content_change_keeps_the_motion_clock_with_a_key_waiting` keeps content continuity, with the explicitly queued second key replaced by immediate retargeting. The title-once and crossing tests follow item 14. `draw.rs` retires none.

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
| departure | `a_finished_jobs_cell_leaves_instead_of_staying_an_empty_box` | When a command's job finishes, its cell slides out and is removed; no frame leaves it as a bordered box with no name. |

Also added: `replaying_the_same_events_and_dt_produces_identical_frames`, `painting_twice_changes_no_layout_state`, `an_unchanged_scan_does_not_extend_travel`, and `claiming_an_entering_vacant_cell_preserves_its_identity_and_admission`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/constants.rs` — the motion code, the painter, `TileGroupTitle` and their tests.
- `crates/tui_pane/src/tiles/target_layout.rs`, `crates/tui_pane/src/tiles/motion.rs` — new: the pure target layout and the step.
- `crates/tui_pane/src/tiles/settings.rs` — the ripple step settings, which the new model drops.
- `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — the additive preparation API and the `TileGroupTitle` export.
- `crates/cargo-tile/src/render.rs`, `crates/cargo-handler/src/render.rs` — the render callers, which prepare a frame before they call `draw_tile_grid`, cargo-handler's `group_title`, and cargo-tile's `MotionColumnWidths` and allocation-aware column fit (items 17 and 18).
- `crates/cargo-tile/src/shim_registration/app_scenarios.rs` — the whole-frame test helpers, where the preparation step reaches them; the in-module tests in `draw.rs`, `grid.rs` and both apps' `render.rs` that call `draw_tile_grid` move with it.
- `crates/tui_pane/CHANGELOG.md` — one Fixed line, one Changed line for the group title, and one Added line for the preparation API.

**Seats:** 1 writer + 1 tester — the items sit in one motion path, so one seat writes.
- `impl` — every source file above: the motion and painter code, the exports, both apps' render callers and their test callers, cargo-tile's column widths (items 17 and 18), and the `tui_pane` changelog; lists each new test that reaches 0.8 s in the phase report.
- `test` — no source file; owns the live fixture, capture and cost lanes. Before any repair it replays the live cases on the phase 20 checkpoint build and writes each item's fixture roles and expected heights for `impl`; then it runs the live capture tools and the per-frame cost comparison on each candidate build.

**Constraints from prior phases:** Phase 14's ruling holds: a piece that is entering or leaving slides, and the spare-row rule binds cells that are neither, and every settled frame. Everything Phase 14's live check met stays met: the emptied cell on the seventh key stands 0 frames above its floor, the summary shows 0 blank rows, no empty cell leaves from its full floor height, one piece per `+`, 0 band escapes. No new row probe or row measurement per moving frame; the seven-cell moving budget stays 29, and speed stays within 1 ms of the Phase 13 build. Three repair rounds in Phase 14 each fixed their fixture and moved the defect to a neighbouring live state (memory `attempts-tile-phase14-motion-rows`): a repair is proved on the live capture before it closes. Phase 22 runs after this phase by the showrunner's landing call and measures the suite it ships. This phase runs after Phases 19 and 20; Phase 19 changes the summary's spare-row taker, its separators and the row order (its items 11, 14 and 15): replay every case on the phase 20 checkpoint build before repairing, and keep Phase 19's shared measurements. The fixture baseline is the phase 20 checkpoint; earlier captures stay as evidence only. Their absolute roots, under `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad`: shots in `ph19`, `ph19b`, `ph19c`, `ph19d` and `ph19e` (each with `shots/`); captures in `p19`, `p19b`, `p19c`, `p19d` and `p19e`, and the motion captures `probe12/keepcap_p19`, `probe12/keepcap_p19b`, `probe12/keepcap_c_p19` and `probe12/keepcap_c_p19b`, with their capture scripts `probe12/cap7c_p19b.sh` and `probe12/cap6_p19b.sh`. `TileCells::readout_rows` and `TileCells::content_extent` keep their answers on the final allocation in the prepared frame; painting triggers no new measurement. Items 5 and 8 may already be partly met (`draw.rs` tests ~2522 and ~2535 cover immediate summary donation): repair only what a replayed live failure shows. The immediate transfers items 5 and 15 permit are each one event for every piece they touch: the resident that gives or moves rows and each mate that takes them may change by more than 3 rows in that frame; the Goal, the tests and the capture tool's counters class them the same way, and the jump gate excepts them. Type design: `HeightSnap::previous_height -> Option<u16>` (`grid.rs` ~367) folds disabled probing into missing history, and `open_waiting_commands_in_current_motion` (~2128) stands a new piece in at height 1; the repair names admission time and height history as states at that boundary, with no bare `Option` or height sentinel as the answer. The shared fixture is `grid.rs::tests::motion_rules::every_frame_of_a_key_keeps_the_motion_rules`. This phase does not edit `.config/nextest.toml`: it lists each new test that reaches 0.8 s, and Phase 22's measurement and exact-name override place it. `draw_tile_grid` keeps its public signature: it becomes a read-only painter, and a frame is prepared before it through an additive preparation API, so drawing writes no layout state. Phase 16 kept `group_title`'s signature; this phase's change to it is the approved exception the Delegation Context names. Phase 17 rebuilt `unchanged_mates_stay_between_their_endpoints_during_plus_and_minus_motion` to sample its stable frames by integer height steps (test helpers `endpoint_painted_rows`, `single_column_height_steps`, `endpoint_rows_drawn`) and to measure every asserted frame, the last included, through the frame preparation; an exhaustive comparison over 1001 frames matched, and it runs in 0.056 s. Its port keeps both: per-frame measurement through the new step, and step sampling instead of a sweep of every raw progress value.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `verify.sh test` for `tui_pane`, `cargo-tile`, `cargo-handler` and `cargo-port`: green, no test at a second or more.
- Live, controlled workload, two rounds, read with the Phase 14 tools: command cells at once 0; pieces vanishing from over 2 rows 0; resident jumps over 3 rows 0 outside the named transfers of items 5 and 15; binding spare-row frames 0 under item 5's ruling; on keys free of arrivals 0 reversals and one motion per key; band escapes 0 on the changing-width capture.
- Speed on a quiet machine: moving and settled medians within 1 ms of the Phase 13 build.
- Shots of the motion keys and a design check by a fresh helper.
- The live frame check against the phase 20 checkpoint build reads no worse on every gate, and per-frame cost on natedev is no more than that build's.
- Item 17, in the unit director's capture: every column offset is constant across each motion's frames except a widening, which happens at most once per column per motion.
- Item 12, in the unit director's capture: 0 reversal flags and one motion on every key that lands within 0.15 s after a command arrives or claims a cell.
- No `pending`, `Transition.source`, `motion_heights`, `retargeted_destination` or `DestinationDepthStatus` remains, and no draw path writes layout state; `draw_tile_grid`'s signature is unchanged.
- `TileCells::group_title` returns no bare `Option`, and the existing tile render tests pass without edits to the titles they assert.
- Every test the Test map keeps passes unchanged; every ported test keeps its visible assertions; every new test passes, `a_finished_jobs_cell_leaves_instead_of_staying_an_empty_box` included.
- Probe tests hold the ceilings: at most 29 row-measurement calls on a seven-cell moving frame, the existing settled budget, and no measurement caused by painting.
- Cost: `test` compares the byte-identical candidate against the phase 20 checkpoint build on a quiet natedev, over event frames as well as steady moving and settled frames; per-frame cost is no higher, and the Phase 13 1 ms gate holds.
- The unit director's notice lists each retired `grid.rs` test with its reason.

### Phase 22 — Slow tests start first  · status: todo

#### Work Order

**Goal:** On each machine, every non-reader test observed at 0.8 s or more in any measurement run starts before every faster non-reader test in a full `cargo nextest run`; the seven reader scenarios keep their existing priority and one-at-a-time rules. The named slow tests were shortened in Phase 17; this phase only orders the rest, and rewrites no test except one: Spec item 6 shortens cargo-port's watcher test (`spawn_watcher_thread_keeps_watcher_guard_alive_until_shutdown`) when its quiet time on natedev is a second or more.

**Spec:**

1. **Measure before.** On each machine run the suite CI runs, `cargo nextest run --all-features --workspace --exclude cargo-mend --tests`, three times with JUnit turned on from outside the repo config: a file `junit.toml` holding `[profile.default.junit]` / `path = "junit.xml"`, passed as `--tool-config-file tile-fixes:<absolute path>/junit.toml`. Each `testcase` then carries its start `timestamp` and `time`. Every run writes the same `target/nextest/default/junit.xml`, so before the next run copy it to `<machine>-<before|after>-<n>.xml` (machine `natedev` or `mac`, `n` 1 to 3, or 4 to 6 for a repeated after batch) in the delegate session directory; the Mac lane copies each file there over `scp` as it finishes, so every run's evidence survives. Record the wall time of each run and, per test, its slowest time. On the Mac work in a clone outside `/tmp`, reached with `ssh mac`: `git clone git@github.com:natepiano/cargo-liner.git /Users/natemccoy/rust/cargo-liner-measure` if it is absent, then before every batch of runs `git -C /Users/natemccoy/rust/cargo-liner-measure fetch origin remove-running-section-tile-fixes`, `git -C … checkout --detach FETCH_HEAD`, and confirm `git rev-parse HEAD` equals the phase tip on natedev and `git status --short` is empty. (`/Users/natemccoy/rust/cargo-liner` is the user's checkout on `main`; leave it alone.)
2. **The slow set** is every test at 0.8 s or more in any of the six runs, less the seven reader scenarios the config already orders. Known members to confirm by name with `cargo nextest list`: in `cargo-tile::unit_tests`, `progress::capture::…::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`, eight in `shim_registration::wire::`, two in `hook::`; two each in `cargo-tile::shim_modes` and `cargo-tile::cli_lifecycle`; in `cargo-handler`, `census::codex::…::reads_threads_by_id_whoever_started_them` and `…::reads_interactive_threads_created_in_the_span`; in `tui_pane`, `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw`.
3. **One override.** Append to `.config/nextest.toml` one `[[profile.default.overrides]]` with `priority = 80` and a `filter` that names each slow test exactly: `binary_id(<id>) & (test(=<name>) | …)` per binary, joined with `|`. No regular expressions, so a test added later is not swept in by accident. A comment above it says what the list is, the threshold, the date measured, and how to re-measure. nextest resolves each setting from the first override that sets it, so the two existing priorities (100, 90), the `cargo-tile-readers` group and both `slow-timeout` entries keep working unchanged; the new override sets `priority` alone and names none of the reader scenarios.
4. **Measure after**, the same three runs per machine. A non-reader test that crosses 0.8 s in an after run and is not in the filter joins it, and the three after runs repeat once on both machines; a second newcomer is reported in the summary rather than chased.
5. **Two reader scenarios still over a second.** `shim_registration::reader_scenarios::reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` (1.70 s) and `reader_keeps_parent_family_across_its_only_childs_source_switch` (1.20 s) measured over a second at load 56 after Phase 17's two repairs. Measured cause: the real reader process's own startup, release to first complete frame 0.51 s and to the CPU rows 0.44 s more, before the fixed 0.25 s observation; the test scaffolding is already overlapped. Measure both in the six runs; where either stays at a second or more on a quiet natedev, record that measured floor in the summary and send the showrunner a proposal for a reader-startup speedup rather than editing the test.
6. **Three more tests over a second under load.** `tiles::draw::tests::a_toast_width_summary_never_draws_compact_with_room_above_its_foot` (tui_pane, 1.19 s and 1.05 s at load 43) and cargo-handler `census::codex::tests::reads_threads_by_id_whoever_started_them` (1.53 s) and `reads_interactive_threads_created_in_the_span` (2.27 s) passed a second in Phase 18's suite runs under heavy load; Phase 18 does not touch them, and the two cargo-handler tests took 0.13–0.14 s in a later run. Measure each in the six runs; for any at a second or more on a quiet natedev, measure its cause, record it in the summary, and send the showrunner a fix proposal, as item 5 does; none of their files is in this phase's Files. Also cargo-tile's `shim_registration::shared_capture::staged_executable_installs_toolchains_and_cleans_up` (1.158 s and 1.321 s at 1-minute loads of 22 to 47 in phase 19's repair gates), under the same rule. Also cargo-port's `watcher::runtime::tests::spawn_watcher_thread_keeps_watcher_guard_alive_until_shutdown` (`crates/cargo-port/src/watcher/runtime.rs`; 1.121 s at a 1-minute load of 248; showrunner routing, 2026-10-13: cargo-port has no other owner in this production): timed on a quiet natedev, shortened in this phase when it takes a second or more there, otherwise its quiet time is recorded and it closes as load. Also, from phase 19's repair round 5 at 1-minute loads of 49 to 62: cargo-tile `render::tests::every_command_cell_shows_a_process` (1.057 s), `shim_registration::wire::matching_relative_directories_keep_their_absolute_paths_and_writer_homes` (1.025 s) and `shim_registration::wire::json_arguments_preserve_both_quiet_spellings_after_separator` (1.134 s), and at loads of 48 to 72 `shim_registration::reader_scenarios::reader_accepts_live_writer_under_different_locale_and_timezone` (1.278 s); time them the same way.

**Files:**
- `crates/cargo-port/src/watcher/runtime.rs` — the watcher test in Spec item 6, only when its quiet time is a second or more.
- `.config/nextest.toml` — one override and its comment. In scope for this unit by the showrunner's word (2026-10-08), though the Units row does not list it.

**Seats:** 1 writer + 1 tester — one configuration owner and two measurement lanes on a byte-identical candidate.
- `impl` — owns `.config/nextest.toml` and, when Spec item 6 shortens it, `crates/cargo-port/src/watcher/runtime.rs`; publishes the `sha256sum` of every changed candidate file on the board after each edit, and runs every natedev before and after run; checks every exact filter name. It also owns Spec items 5 and 6's quiet evidence: it times each named test on natedev with the 1-minute load below a quarter of the cores, records that load beside each time, and gives each test a disposition.
- `test` — no source file; copies every changed candidate file to the Mac clone after the checkout (`scp` over `ssh mac`: the config into `/Users/natemccoy/rust/cargo-liner-measure/.config/nextest.toml` and, when it changed, the watcher test file into `/Users/natemccoy/rust/cargo-liner-measure/crates/cargo-port/src/watcher/runtime.rs`), verifies each file's sha256 against the board before both after-run batches, runs every Mac run, and hands `impl` the named JUnit files, the slow set and the start order. The before runs use the clean phase tip.

**Constraints from prior phases:** This phase runs after Phases 1 to 21, every phase planned to add tests or change shared grid code, so its timings measure the suite those phases ship. It runs last by the showrunner's packaging call (2026-10-12), after Phase 21's last checkpoint when that phase lands in several, so its override names the suite Phase 21's motion rewrite leaves. Phase 21 lists each new test that reached 0.8 s in its report; this phase confirms each by name and by measurement. Phase 17 shortened several of item 2's known members, and Phase 21 replaces most grid and draw tests, so item 2's known list is confirmed against the six runs and `cargo nextest list`, never assumed: a known member under 0.8 s in every run stays out of the filter. Exact-name discovery, the reader priorities and group, and both timeouts hold as item 3 states. Phases 1 to 16 and 18 to 20 added render tests to `tui_pane` and `cargo-tile`; each gate asked for under a second for its own tests, and this phase's line is 0.8 s, so the six measurements decide whether any of them joins the slow set. Seen over a second during Phase 8's runs under load: `tui_pane` `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw` (1.52 s) and several reader and `shim_modes` scenarios.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, after the config edit: green.
- When `crates/cargo-port/src/watcher/runtime.rs` changes: `bash ~/.claude/scripts/delegate/verify.sh lint cargo-port` once, then `... test cargo-port` green, and `spawn_watcher_thread_keeps_watcher_guard_alive_until_shutdown` passes under a second on a quiet natedev.
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, the source-switch reader is first, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.
- The summary gives each test named in Spec items 5 and 6 its quiet natedev time and load, and for any at a second or more, its measured cause, its floor and the fix proposal sent to the showrunner.
