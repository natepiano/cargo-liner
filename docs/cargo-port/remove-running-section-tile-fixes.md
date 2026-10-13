# tile-fixes

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-tile draws a moving cell on the painted ground, keeps every label and readout whole or marked, follows the system's light or dark setting, shows the summary alone when the window is too small for cells, sizes the summary to what it asks, and nextest starts the slow tests first.

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

### Phase 16 — Readable inks, and a foot that fades  · status: todo

#### Work Order

**Goal:** Every ink on the grid meets its contrast floor on both grounds of every built-in theme, a finished row stays readable through the first half of its fade, and a fading cell's foot, and the summary's, fades with its contents.

**Spec:**

Evidence: shots in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, text beside each in its `.txt`. Contrast is measured from cell colours, never PNG pixels (Phase 7 gotcha). Reproduce each item in a failing test before changing anything.

Inks (built-in themes Default Dark and Default Light, transparent off; grounds are the ones `tui_pane` paints, `pane_background(true)` and `pane_background(false)`, `crates/tui_pane/src/pane/chrome.rs`; shots read #0e0e11 and #2e2e39 dark, #e0e0e9 light summary):
1. **Pid identity colours** (`crates/cargo-tile/src/theme/mod.rs` `FAMILY_COLORS` 25, `family_color` 105: one solarized set for both appearances; light 2.44 to 3.47:1, dark magenta 2.95:1 on the summary grounds). Rule: every colour `family_color` can hand out measures 4.5:1 or more on both grounds of the active theme, and the clearance and more-than-one-family tests still pass. One set cannot meet it in both appearances (it would need relative luminance ≥ 0.30 on #2e2e39 and ≤ 0.13 on #e0e0e9), so the set follows the appearance. Test: `every_family_colour_meets_contrast_on_both_grounds_in_every_built_in_theme`.
2. **The red count in the foot readout** (`crates/tui_pane/src/tiles/draw.rs` `rows_readout_lines` 539 writes the count in `error_color()`; Default Dark `error = Red` (`crates/cargo-tile/src/theme/builtins.rs:86`) draws #cd0000: 3.30:1 on #0e0e11, 2.30:1 on #2e2e39). Rule: the over-budget count, and the `@ W` width written in the same ink, measure 4.5:1 or more on both grounds in every built-in theme of cargo-tile, and of cargo-handler, which draws the same readout (`crates/cargo-handler/src/theme.rs:370`). Test: `the_readout_count_meets_contrast_on_both_grounds_in_every_built_in_theme`, one in each app.
3. **Light summary green** (Default Light `success = Rgb(0, 120, 0)`, `builtins.rs:136`: 4.33:1 on #e0e0e9; the shot read 4.05:1 from pixels). Rule: every ink a table row draws at fade 0 (`text_default`, `label_color`, `success_color`, `warning_color`, `secondary_text_color`) measures 4.5:1 or more on both grounds in every built-in theme. Test: `table_inks_meet_contrast_on_both_grounds_in_every_built_in_theme`, next to `status_line_text_meets_contrast_in_every_built_in_theme` (builtins.rs 436, whose `contrast_ratio` it reuses).

Fade:
4. **Dimmed finished rows** (`crates/cargo-tile/src/roster.rs` `faded_at` 137 carries a finished row toward its ground evenly over `DEFAULT_FADE_SECONDS`; the summary's #607374 on #2e2e39 is that blend about half way, 2.95:1). Rule (author's call; the fade stays an exit): a finished row's ink stays at 3:1 or more on its ground until half of the fade has run, then reaches the ground at the end; summary and cells alike. Measured against the inks items 1 to 3 land. Test: `a_finished_row_stays_readable_through_the_first_half_of_its_fade`.
5. **A fading cell's foot fades with it** (`draw.rs` `draw_cell` 415 → `draw_rows_readout` 502 writes full-strength inks; deep L59–L69 show a near-black cell with a bright `content rows: 10  r/c: 9/124`). Rule: every span of the foot readout, and of the summary foot, is drawn as `blend_color(ink, ground, fade)` with the cell's own fade and ground, the same fade its contents use. `TileCells` gains one provided method returning the cell's fade as a named state, `CellReadoutFade::{Unfaded, Fading(CellFadeProgress)}` with `CellFadeProgress` a named amount, never an `Option<u8>`; its default is `Unfaded`, so cargo-handler (`crates/cargo-handler/src/render.rs`) and `draw_tile_cell` keep today's drawing with no edit. cargo-tile answers a command cell from the one function `draw_group` uses (`heading_fade(&rows).min(group.lead.faded())`, render.rs 959) and the summary from `heading_fade` over `summary_rows(...)`, the rows the summary draws. Existing `Option` states in `TileCells` this item does not touch (`group_title`) stay as they are. Tests: `draw.rs` `a_fading_cells_readout_fades_with_it` through `draw_tile_grid` with `StubCells`; `render.rs` `a_finished_commands_cell_reports_the_fade_its_contents_use` and `the_summary_foot_fades_with_its_rows`.

Changelogs: one Changed line each under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md` and `crates/tui_pane/CHANGELOG.md`, and in `crates/cargo-handler/CHANGELOG.md` when its theme changes.

**Files:**
- `crates/cargo-tile/src/theme/mod.rs`, `crates/cargo-tile/src/theme/builtins.rs`, `crates/cargo-tile/themes/*.toml` — family colours as theme roles, inks, contrast tests.
- `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/*.toml` — readout red, its test.
- `crates/cargo-tile/src/roster.rs` — fade curve and its test.
- `crates/cargo-tile/src/render.rs` — the cell and summary fade answers, their tests.
- `crates/tui_pane/src/tiles/draw.rs` — `CellReadoutFade`, the provided method, the faded foot, render test.
- `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — the exports of `CellReadoutFade` and `CellFadeProgress`; `test` owns both.
- `crates/cargo-tile/src/constants.rs` — any new literal.
- `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md` — Changed lines.
- `crates/tui_pane/src/wrap.rs`, `crates/tui_pane/src/toasts/body.rs` — only when the owed frame check reads worse: the repair and its test.

**Seats:** 2 writers — theme data and contrast tests, and the fade path from roster to foot.
- `impl` — items 1 to 3: `crates/cargo-tile/src/theme/`, `crates/cargo-tile/themes/`, `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/`, the cargo-handler changelog. Posts on the board when the inks are final.
- `test` — opens as impl; items 4 and 5: `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/render.rs`, `crates/tui_pane/src/tiles/draw.rs`, the cargo-tile and tui_pane changelogs; hub: `crates/cargo-tile/src/constants.rs`. Does item 5 first; starts item 4 only after `impl` posts its inks final, because item 4 measures them.

**Constraints from prior phases:** Theme installation does not keep the resolved `Appearance`, so `family_color` cannot ask it: encode the family hues as cargo-tile theme roles in `builtins.rs` and each theme file, so each theme carries its own set and the active theme selects it. Phase 4: no ink changed then; any ink change now is checked on both themes, transparent on and off. `theme/state.rs` keeps its four `#[expect(clippy::expect_used)]`. Phase 7: `status_line_text_meets_contrast_in_every_built_in_theme` holds status inks at `MIN_STATUS_LINE_TEXT_CONTRAST` = 4.5, and named colours resolve through its xterm table. Phase 14 makes column offsets frame-wide and insets every row one cell from the right frame. New public items are only `CellReadoutFade`, `CellFadeProgress` and the provided `TileCells` method; no existing `tui_pane` public signature changes. cargo-handler's cell sizes do not change. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- Owed from phase 15 (the showrunner's landing call, 2026-10-12): phase 15 was checkpointed with its live frame check not run, because a build hold blocked every capture. It runs at the first build-hold status reading `no build hold`, on phase 15's checkpoint binary, judged against phase 14's numbers; the result goes to the showrunner either way, and a worse result is repaired before this phase's checkpoint. Inputs, all under `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/`: the helper's brief `live15_prompt.txt`, the exact commands in `probe12/HANDOFF_live15.txt`, the comparison script `probe12/g15.py`, phase 14's numbers in `probe12/out/results_p14.txt` (section `P14d`), and the binary `ph15/cargo-tile`. The unit director runs it through a helper; the helper's last line reads `PHASE 15: NO WORSE THAN PHASE 14` or `PHASE 15: WORSE ON <gates>`. A worse result is a repair of phase 15's own files by seat `impl`, who wrote them, with a failing test first: `crates/tui_pane/src/wrap.rs` and `crates/tui_pane/src/toasts/body.rs`, then `test tui_pane` and `test cargo-tile`, because cargo-tile's tables use the same wrapper.
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- Unit director's captures, the eight shots, on both themes, transparent off and on: every measured ink at its floor from cell colours, and a fading cell's foot as dim as its contents.
- A fresh helper's design check of those shots: pass.

### Phase 17 — One reading per command, one ancestry form, and a titled summary  · status: todo

#### Work Order

**Goal:** Each command shows its own progress once, ancestry marks a cut one way, the Short view keeps the subcommand, marks dropped arguments and names one session runner by file name, the summary title has its blank cell, and no column moves when a value changes length.

**Spec:**

Evidence: shots in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, text beside each in its `.txt`. Reproduce each item in a failing test before changing anything. `render.rs` line numbers are HEAD `20a21b7e`; find functions by name.

1. **Each command shows its own progress** (`render.rs` `heading_gauge` ~2939 draws the first `Working` row's reading on the directory heading). In the summary, two `cargo check` runs in obsidian_knife (deep L12–L14) both read 84%, while one's cell reads 58% (deep L76). In a command cell, the second heading `[hana-linux-1] unavailable` repeats the lead's 99.8% gauge (deep L22, L26). Rule: every percent on screen belongs to one command, and each command with a reading shows its own. A heading carries a gauge only when one command in its group has a reading. When two or more do, each such row carries its own `percent_reading` in its `state` cell, which joins for that case (author's call). A group whose rows read only through an enclosing capture already shown in the cell (`RowProvenance::Enclosing`) draws no gauge. Tests: `two_commands_in_one_directory_show_their_own_progress`; `a_heading_does_not_repeat_its_cells_progress`.
2. **One ancestry form** (`render.rs` `ancestry_levels` ~1493, `ancestry_fit` ~1369, `ancestry_lines` ~1602). Today one row draws `… <nearest>` inline and two or more draw a separate `…` row (deep L42, L52, L62, L72). The inline `…` sits in an unstyled span, so it draws white (#ffffff) on a dimmed row (#616163). Rule: at every budget, a cut is marked inline at the head of the first level drawn after it (`… <pid> <command>`), and the root is kept when the budget is two or more. `AncestryLevel::Elided` goes. The mark takes the same faded ink as that level's command text. Tests: `an_ancestry_cut_is_marked_inline_at_every_budget` (budgets 1 to 4 over a six-level chain), `the_ancestry_mark_takes_its_rows_faded_ink`. Phase 7's ancestry-level tests change only where the form changes, each named in the summary with old and new text. After item 5 a cut happens mostly in Long view, but Short view's fallback (no qualifying session runner, drawn as today) can still cut, so this one form applies in both views. **A cut ancestry gives up its gap row** (`ancestry_budget` always subtracts `ANCESTRY_GAP_HEIGHT`). Today a cell whose ancestry hides levels still keeps the blank row between ancestry and table: deep126x80 L24, L41, L49, L58, L66, L74; toast200 L38 (phase 10 design check). Phase 12 already gives a lone available row to ancestry with no gap (`ancestry_allocation` ~1433; tests `one_spare_row_draws_ancestry_without_a_gap`, `one_spare_ancestry_row_does_not_have_to_be_kept`), and that stays. Rule: when the ancestry hides any level at a budget of two rows or more, the gap row gives way and carries one more level, like the gaps between directories; a whole ancestry keeps its gap. Test: `a_cut_ancestry_gives_its_gap_row_to_one_more_level` — a six-level chain at budgets 1 to 3: no blank row between the last level and the `pid` header; budgets 2 and 3 fail before the change, budget 1 already passes.
3. **Short view keeps the subcommand after a flag** (`crates/cargo-tile/src/census/command_text.rs` `named` 115 stops at the first flag via `names_the_command` 191, so nextest's inner `cargo --… run` draws as a bare `cargo`: w200 L32, deep L37, L48). Rule: `named()` skips flags, and the value of a flag in `SUMMARY_HIDDEN_VALUED_FLAGS`, and keeps the words that name the command after them. When the Short view drops any argument, the name ends in ` …`, the same mark the ancestry rows use (`cargo check …`); with nothing dropped it ends at the name (`cargo check`). The user decided this, 2026-10-09 (via the showrunner). Phase 14's drop-before-wrap rule measures `named()`, so this item changes what it measures, the mark included. Tests: `the_short_name_keeps_a_subcommand_after_a_flag` (`--color always nextest run` → `nextest run …`; `--locked check` → `check …`); `the_short_name_marks_dropped_arguments_only_when_it_drops_some` (`cargo check --workspace` → `cargo check …`; `cargo check` → `cargo check`).
4. **One blank cell after the summary title** (`crates/cargo-tile/src/constants.rs:166` `SUMMARY_CELL_TITLE = " summary"`, written verbatim by `GridLines::add_titled`, `crates/tui_pane/src/pane/frame.rs:481`; every shot L1 reads `┌ summary───`). Rule: one blank cell before and after the title, as the sccache label and toast title have. cargo-handler's own summary title is out of this phase (author's call: the approved evidence is cargo-tile's). Test: `the_summary_title_has_a_blank_cell_after_it` — the top row reads `┌ summary ─`.
5. **Short view names the one ancestor that ran the session** (`render.rs` `drawn_ancestry` and `shortened`, which today keep the whole chain in Short view and only drop arguments; `census::command_name`). Evidence, the user's shot `/tmp/claude-1000/-home-natepiano-rust-cargo-liner/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/images/2.png`: a hana-linux-2 cell whose Short-view ancestry runs `Runner.Listener` (519409), `Runner.Worker spawnclient` (1556662) and `bash` (1558245), each a full `/nix/store/…` path wrapping over two or three rows. The user, 2026-10-09 (via the showrunner): "In compact mode it's not helpful to see the full stack of items that drove the session - it woul be ideal to just show the command that most likely was the important session runner - maybe that's not the last if it's always bash, right?" Rules, set by the showrunner:
   - In Short view the ancestry block is one row: the nearest ancestor that is neither a shell nor a pure exec wrapper. Shells: today's `TRANSPARENT_PROCESS_NAMES` (`crates/cargo-tile/src/constants.rs`, which includes `login`, so a terminal session's login is never the one row; showrunner, 2026-10-09) plus `nu`, kept as that one list rather than a second copy. Wrappers: `env`, `nice`, `nohup`, `setsid`, `stdbuf`, `timeout`, `time`, `sudo`, `doas`, `script`, `ionice`, `chrt`, `taskset`, `caffeinate` (showrunner's ten plus four, accepted), a new list in the same file. Both are matched on the program's file name. When no ancestor qualifies, the block is drawn as today. Adding `nu` to the shells reaches the census (`crates/cargo-tile/src/census/scan.rs` reads `TRANSPARENT_PROCESS_NAMES`); test `nu_is_a_transparent_launcher`.
   - In Short view a program is named by its file name, never its path, in ancestry rows and in table rows alike: `/nix/store/…/Runner.Worker spawnclient 166 175` reads `Runner.Worker …`. The ` …` follows item 3's mark rule.
   - Long view keeps the whole chain with full command lines, unchanged.
   - Short view gives up the foot's pid link to the first row's parent when a shell sits between them; Long view keeps it (showrunner's call).
   Write the failing test from the shot's chain first. Tests: `the_short_ancestry_is_the_nearest_session_runner` (the shot's chain `Runner.Listener` → `Runner.Worker spawnclient 166 175` → `bash` → `cargo` gives one row `1556662 Runner.Worker …`; `claude` → `bash` → `cargo` gives `claude`; `ghostty` → `zsh` → `cargo` gives `ghostty`; `env` and `nohup` between them are skipped too); `a_short_view_program_is_named_by_its_file_name` (an ancestry row and a table row whose program is a `/nix/store/…` path); `the_long_ancestry_keeps_every_level` (the shot's chain in Long view, unchanged).
6. **A cell's own command comes first** (phase 10 design check, toast200 L42: the hana_catalyst-lfo cell, cut to one row, shows a child `909889 909862 … cargo` that started 0 s ago, while its own `909862 cargo nextest run`, the row the summary names and the cell's first row in deep126x80 L45 and toast64 L37, hides behind `…`). Rule: in a command cell, the cell's own command row (the lead) is drawn first and its children follow, so a cell cut to one row shows its own command (author's call). Test: `a_cell_cut_to_one_row_shows_its_own_command`.
7. **One way to cut a directory heading** (phase 10 design check, w40 L3 `[natepiano] …/tool-based-ui-trunk` against L7 `…/rust/tool-based-ui-frame-time`, which is 31 cells and fits a 36-cell line whole; toast40 L3 against L11). Rule: a heading drops its `[account]` tag first and shows the path whole when it fits; only a path that does not fit alone loses leading segments behind `…` (author's call). Test: `a_heading_drops_its_tag_before_cutting_its_path` — at inner width 38: `~/rust/tool-based-ui-frame-time` whole, `…/tool-based-ui-geometry-material` cut.
8. **A chain cut by the walk limit shows its cut** (phase 13 design check, deep126x80 rows 30–55: one cell's ancestry starts at `1914627 uv run …` at indent 0, while the cells above and below show the same `uv run` as the fourth level under `3334 systemd`, `27285 tmux` and `2273637 claude`. Three root levels are gone with no `…`, while the cell keeps 2 blank rows above its foot and its count reads whole. Cause read in code: the parent walk stops at `PARENT_WALK_LIMIT = 32`, `crates/cargo-tile/src/constants.rs` ~341; confirm it in a failing test first.) Rule: a chain the walk limit cuts opens with the cut mark in the one ancestry form item 2 sets, inline at the head of its first drawn level (`… <pid> <command>`), in Short and Long view alike. The walk says why it stopped: `ancestor_pids` (`crates/cargo-tile/src/census/scan.rs` ~1655) returns a named outcome, the chain ended or the limit cut it, in place of a bare vector, and `TrackedGroup` (`crates/cargo-tile/src/roster.rs` ~170) carries it to the drawing. The cut is never inferred from how many levels are drawn, because filtering cargo plumbing and unreadable processes shortens a chain too. The foot says cut too (showrunner, 2026-10-11): a cell whose chain is marked cut never reads as whole in its foot. Today `rows_readout_lines` (`crates/tui_pane/src/tiles/draw.rs` ~531) colours the `content rows:` count from one comparison, the rows against the content height, so a cut chain whose known levels all fit reads whole. `TileCells` gains one provided method returning a named state, `CellContentExtent::{Measured, CutAtSource}`, default `Measured`; cargo-tile answers `CutAtSource` for a cell whose chain the walk cut, and the readout draws that cell's count in the cut colour at any height. The state changes the foot's colour and nothing else: row counts, demands, spare-row sharing and moving heights read measured rows alone, so no row is invented for levels nobody measured. Tests: `a_chain_cut_by_the_walk_limit_opens_with_the_cut_mark` — a chain with one more parent beyond the limit opens with `… <pid> <command>`; a chain that ends exactly at the limit draws no mark; a cut chain that filtering leaves shorter than the limit still opens with the mark; `a_cell_cut_at_its_source_never_reads_whole` (`tui_pane`, render then read the buffer) — a cell with room for every row and `CutAtSource` draws its count in the cut colour, and the same cell with `Measured` draws it in the whole colour; `a_walk_cut_chain_reads_cut_in_its_foot` (cargo-tile, through `Cells`, Short and Long view) — every known level fits, the mark is drawn, the foot count is in the cut colour, and the cell asks for no row beyond the ones it draws.
9. **`cpu` and `mem` keep one width for the session** (phase 14 design check, `motion_01_plus` against `motion_02_plus` one second later: `cpu` is 5 cells for `1017%` and 4 for `765%`, so every column to its right moves one cell: cell `mem` 40→39, `command` 46→45, `compiler` 67→66, `runs` 78→77; summary `mem` 32→31, `command` 38→37). Today both of Phase 14's frame grids fit each column to the widest value drawn that frame. Rule (showrunner's call, 2026-10-11): `cpu` and `mem` each take one width for the session, sized for the widest value the machine can report (cores × 100% and total memory, each in the form its column draws), so no header or column moves when a value gains or loses a digit, in the summary or in any command cell. The two widths are read once per session, never per frame, and enter Phase 14's frame grids as an input, never as a second fitting beside them; the drop rule still drops either column whole. Tests: `a_value_changing_length_moves_no_column` (render two frames whose `cpu` reads `765%` then `1017%` and whose `mem` gains a digit; every header offset in the summary and in each command cell is equal across the two frames); `cpu_and_mem_are_sized_for_the_machine` (the two widths for a stated core count and memory total). Phase 14's second design check saw the same shift from `compiler`: `runs` moves between columns 78 and 81 when `sccache×N` gains a digit. Before dispatch the unit director asks the showrunner which rule `compiler` takes, and this item carries it. Ruling (showrunner, 2026-10-12): a group's `mem` sums its processes' resident memory and can exceed total memory (`Census::attribute_memory`, `scan.rs` ~1158); when a value needs more cells than the column has, the column widens once to the widest value seen this session and never narrows again, and the reading is never altered to fit. Test: a summed reading crossing its digit boundary widens the column once and a later smaller reading leaves it.
10. **The hidden-rows mark is drawn in neutral ink** (phase 14 design check on the repaired build: `w40`, `w40_cut`, `deep200x30`, `v77_cells`, `toast200`, `motion_04` to `motion_08`). The `…` that marks hidden rows in a command table takes the style of the command's last drawn cell (`mark_table_continuation`, `buffer[(right, y)].style()`), the argument green, while ancestry marks are neutral (220,220,220). On the frame's longest command it stands one blank cell after the last letter (`cargo nextest run …`), so it reads as a cut argument list. Rule (showrunner's call, 2026-10-11): the hidden-rows `…` is drawn in the neutral ink the other marks use, never the command's colour. Take that ink from the style the ancestry mark already uses, through one shared name where the code allows, so no site picks its own. Test: `the_hidden_rows_mark_is_drawn_in_the_marks_ink` (render a table that hides rows below a whole command; read the mark's cell colour back from the buffer and compare it with the ancestry mark's).
11. **The summary's spare row goes to a cell that hides an ancestry level** (phase 14 design check on the build after its second repair round: `w40` line 14, `w48` line 11, `toast40` line 11). The summary draws one blank row above its foot while cells in its column hide ancestry levels behind `…`. Phase 13's retrospective records the cause: the retained-row answer (`rows_kept`) used as the receiving-step answer counts an optional ancestry row a command paints as no increase, so the row has no taker. Rule (showrunner's call, 2026-10-12): the summary's spare row goes to a cell that hides an ancestry level. A cell that hides an ancestry level is a taker of a column mate's spare row, in settled frames and in motion; phase 14's rules still hold (no blank row to keep moving, heights one way within a key, no added row probe per moving frame). Files: `crates/cargo-tile/src/render.rs` (`rows_kept` and the shared measurement); if the receiving step needs an answer of its own in `crates/tui_pane/src/tiles/grid.rs`, the unit director adds that file to a seat at dispatch. Test, on the render path: `the_summary_gives_its_spare_row_to_a_cell_hiding_an_ancestry_level`.
12. **A heading's `…` marks only a cut that removes characters** (same design check: `w40` lines 9 and 38, `toast40` lines 6 and 35). A heading whose owner tag is dropped reads `…/rust/tool-based-ui-frame-time`, where `~/rust/tool-based-ui-frame-time` is the same width, so a whole path reads as cut. Rule (showrunner's call, 2026-10-12): `…` marks only a cut that removes characters; when dropping the owner tag alone makes the path fit, the heading draws the path whole from `~`. Test: `a_heading_that_fits_without_its_tag_is_drawn_whole`.
13. **An argument mark stays on its command's row** (phase 14's full design check: `w48` line 17). An ancestry row, `1158012 /run/current-system/sw/bin/python3`, fills its row exactly, so its argument mark wraps onto line 18, a row that holds only `…`, in a cell that hides 3 rows. Rule (showrunner's call, 2026-10-12): the argument mark stays on the command's own row, and a wrap row never holds only `…`. Item 4's file-name rule removes this instance; the rule holds for every row that carries a mark. Test: `a_wrap_row_never_holds_only_the_mark`.
14. **A key that lands while a command's motion runs joins that motion** (`crates/tui_pane/src/tiles/grid.rs`: `add`, `remove`, the pending queue, `open_waiting_commands_in_current_motion`; moved here from phase 14's live check by the showrunner's landing call, 2026-10-12). Evidence, the controlled workload (200x50, cells view) on phase 14's build after its second repair round: a `+` that lands 7 to 112 ms after a command claims a cell waits 0.96 to 1.08 s for its slot, because the key queues behind the command's motion. In that wait the summary takes 4 rows and gives them back when the slot enters (second round, sixth `+`: height 13 to 17 at +0.451 s, 17 to 13 at +1.077 s, 13 to 11 at +1.438 s), and a command cell takes one row and gives it back (third `+`, both rounds: 5 to 6 at +0.281 s, 6 to 5 at +1.421 s). Rule (showrunner's call, 2026-10-12): a key that lands while a command's motion runs joins that motion, and no piece takes a row it gives back. Phase 14 already does this for a command whose cell opening is still queued; this item covers the command that has claimed an existing cell or whose cell has entered. Tests, on the render path: `a_key_landing_after_a_command_claims_a_cell_joins_its_motion`, `no_piece_takes_a_row_it_gives_back_when_a_queued_key_enters`. Gate: the unit director's capture reads 0 reversal flags and one motion on every key that lands within 0.15 s after a command arrives or claims a cell.
15. **The summary's separator rows give way before a cell hides a process row** (phase 14's design check on its checkpoint build: `w40` lines 5, 8, 11 and 15 against lines 32 and 42). Settled, at 40 columns: the summary keeps four blank rows between its projects while two cells in its column each hide a process row behind `…`. The summary has a compact form without those rows (`v77_cells`). Rule (showrunner, 2026-10-12): the summary drops its blank rows between projects, earliest first, before any cell hides a process row. Mechanism: `ProcessTablePlan::keep_gaps` keeps every separator or none today, and the summary's row measurement reports kept = drawn with the separators in it (`render.rs` ~437, ~2599, ~2637), so this needs the table plan and the row allocation changed together: one shared measurement that names which separators stand. Tests: a summary gives some separators and keeps the rest when a mate needs fewer rows than it has; separators go earliest first; all separators stand when no mate hides a process row.
16. **A parent's row stands before its child's, and is the row kept under a cut** (same design check: `v77_cells` lines 22 and 27, `w40` line 42, `toast200` line 45, `deep126x80` line 75). A child `cargo` row stands above the row whose pid is its parent, and where rows are cut the child is the one kept: `tool-based-ui-trunk` draws only `3208428 3208354 cargo …` while the summary names its parent, `cargo nextest run`. Other cells list the parent first. There before Phase 14. Rule (showrunner, 2026-10-12, placed in this phase): a parent stands before its child, so under a cut the kept row is the command the summary names. `group_by_path` sorts by `(started, pid)`, which does not give this. Directory groups keep today's order; the rule orders rows inside a group. Tests: a parent that is not the group's lead stands before its child; a child whose parent sits in another directory group leaves both groups' order unchanged.
17. **A command names its profile when it is not the default** (the user's requirement, relayed by the showrunner through hana on 2026-10-12: "for the command we show in the summary and in the cells - it is important to show that it's a release if that option was invoked - don't show debug as it is the default - but if some other profile is invoked then yes, we'd want to see it"). A release run (`--release`, `-r`, `--profile release`, `--profile=release`) always shows that it is a release, in the summary row and in the command's cell. Any other named profile shows too. The default dev profile never shows: an explicit `--profile dev` is dropped from the shown line like the noise flags. The mark survives every width cut: a row that cuts its command still shows the profile. One form in both places; its spelling and placement are the unit director's call under the style guide, stated in the phase report. `SummaryDetail::Trimmed` keeps `--release` in the text today, so first find where the user loses it (a width cut that drops the command's tail is the lead) and name that place in the phase report. Read before dispatch, 2026-10-12: two places lose it. The Short process tree draws `CommandText::named()` (`render.rs` ~2085, ~2941, ~3045), which keeps words only up to the first flag (`census/command_text.rs` `named`, `names_the_command`), so `build --release` draws as `build` at any width. `CommandText::line()` keeps `--release` under both `SummaryDetail` values (it drops only `SUMMARY_HIDDEN_VALUED_FLAGS`), so the Long view and the summary lose it only where a row cuts the command's tail. The seat confirms both in failing tests first. The census reads the profile from cargo's own arguments only, up to the `--` that ends them, so a child program's `--release` or `-r` is never read as cargo's. `CommandText` hands the drawing the profile as a named state (such as `CargoProfile::{Default, Release, Named(..)}`, never an `Option<String>`), and both views draw the mark from that one state; seat `test` owns it in `census/command_text.rs` and publishes it on the board before `impl` draws it. Tests: `cargo run -- --release` shows no mark; a release run shows the mark in the summary row and in the cell under each of the four spellings; a named profile other than dev shows; `--profile dev` and no profile flag show none; the mark is still drawn at the narrowest width that cuts the command, in the summary row and in the cell.
18. **The capture toast's title is in plain words** (phase 15's design check on its checkpoint build, 2026-10-12: `toast200` line 43, `toast64` line 43, `toast40` line 41). The card's title still reads `Capture shim installed` while its body no longer names the shim. The title says what the body says, in the body's words: `Capturing cargo progress` (the showrunner's ruling, 2026-10-12: it uses the body's own verb). Site: `crates/cargo-tile/src/capture.rs` line 56, and its test at line 189. No other toast title changes.
19. **A column leaves only when a drawn row cannot hold it** (same design check: `w40_cut` line 46, and `w40` lines 29 to 30 against `toast40` line 21). Two cases. First: at 40 columns with the summary cut, the summary draws only `pid` and `command`, though `pid cpu mem command` with its mark takes 30 of the 36 cells. A row hidden below the cut, with a 20-cell command, sizes the command column and drops `cpu` and `mem`. Rule: only rows that are drawn size the frame's column grid and decide which columns drop. Second, cause not found: `w40` draws `pid parent command` in 30 of 36 cells with no `cpu`, while the 40-column toast frame draws `cpu` in 33 cells. Lead, not verified: `FrameTableColumnGrid::columns_for_command_name` (`render.rs` ~2253) budgets the summary's drops against command-cell slots. The seat reproduces each case in a failing render test first, and reports the cause of the second before repairing it. Phase 14's grids stand: `FrameColumnGrids` (`render.rs` ~2133) holds two independent grids, one over the rows of every command cell and one over the summary's rows. Command tables still share one set of column offsets, and the summary keeps its own. How the first case is built (showrunner's ruling, 2026-10-12): rows this frame can never draw do not size a grid. `FrameColumnGrids::new` takes the frame's inner height, and a summary row past that height is left out of the summary's grid; there is no second pass and no state between frames, so demands, drawing and row probes still read one immutable result. What this leaves: a row hidden by a smaller allocation still sizes the grid; a shot that shows it goes to Phase 20 as a two-pass follow-up. Tests for the first case: a summary whose last project lies past the frame's height, with a 20-cell command, still draws `cpu` and `mem`; a row inside the frame's height with that command still drops them.
20. **The ancestry block hides only the levels it must** (same design check: `w48` lines 25 to 35). A cell hid two levels (`tmux`, three rows, and `claude`, one row) behind the mark while one row stood blank above its foot; hiding the three-row level alone fills the cell exactly. Cause, read in the code on 2026-10-12: `ancestry_fit` (`render.rs` ~1500) lowers a count of levels until the block fits, and the mark takes one of the counted places. So the form that hides one level of a five-level chain has the count of the whole chain, and is never measured. That held while every level took one row; a wrapped level takes more. Built with item 2 as one change by seat `impl`: item 2 draws the mark inline and removes `AncestryLevel::Elided`, so the mark no longer takes a counted place and every form is measured. Rule: the fit is in rows, not levels. It keeps the root when the budget is two rows or more, then as many of the levels nearest the table as the budget holds, with item 2's inline mark at the head of the first of them. Each candidate is measured with `ancestry_height` on the levels as they will be drawn, mark included, because the mark's cells can push a level onto one more row. The parent of the table's row is always the last level kept, and item 2 sets the forms at budgets of one and two. A level is hidden only when the rows left cannot hold it, so a blank row may stand above the foot only when the hidden level nearest the table needs more rows than are left. Tests: a chain of heights 2, 3, 1, 1, 1 at a budget of 6 keeps the root and the three levels nearest the table and hides only the three-row level, which the one row left cannot hold (today's code hides two levels); a chain of heights 1, 1, 2, 1, 1 at a budget of 5 hides only its second level and fills the budget exactly; a chain of one-row levels draws item 2's form; a budget past the chain's length still measures the whole chain once (the cost `ancestry_fit`'s comment names).

**Pending decision: what a heading shows when its path cannot be read; and whether text with double-width characters wraps by the cells it takes**

These are two add-ons that change what the user sees, so each answer is the user's and arrives through the showrunner (showrunner, 2026-10-12). The answers, for each one: build it in this phase, put it on the next-items list, or drop it. This phase does not dispatch while the block stands.

Actual problem:
1. A directory heading whose path cannot be read draws `[hana-linux-2] unavailable testing` and repeats the progress bar and `22.2%` of the heading above it; in a narrow column it is cut from the front as `…vailable` (cargo-tile directory headings in `crates/cargo-tile/src/render.rs`).
2. `cell_count` and `split_at_cells` in `crates/tui_pane/src/wrap.rs` count characters, not terminal cells, so a word or a nonbreaking run holding double-width characters is measured at half its drawn width and a line can be clipped with no mark. The counting is older than Phase 15, whose nonbreaking runs now promise a mark.

What exists now:
- First: Found in Phase 7. Shots `P/ph7/shots/deep126x80.png` row 33 and `w200.png`.
- Second: Found by Phase 15's plan review. No shot: none of the run's captures holds double-width text.

What should change:
- First: Such a heading shows the runner label alone and never repeats its parent's progress; a render test pins it.
- Second: The wrapper measures and splits by terminal cells without splitting a grapheme; tests with double-width text through `wrapped`, `wrapped_runs` and the toast renderer.

Recommendation:
1. Build it in this phase: it is drawn wrong in normal use, and this phase already repairs headings in the same file.
2. Put it on the next-items list: the commands, paths and toast text this tool draws have been ASCII in every capture, and the change adds width measurement and grapheme splitting to the wrapper every table uses. Only the user knows whether wide text appears in their projects; if it does, build it in this phase, which is the last one allowed to edit `wrap.rs`.

Changelog: one Changed line under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/cargo-tile/src/render.rs` — progress per command, ancestry form, the Short-view session runner, the session widths of `cpu` and `mem`, tests.
- `crates/cargo-tile/src/census/command_text.rs` — `named` and the program's file name, their tests.
- `crates/cargo-tile/src/census/scan.rs` — the `nu` classification test, and item 8's walk outcome.
- `crates/cargo-tile/src/roster.rs` — `TrackedGroup` carries the walk outcome.
- `crates/cargo-tile/src/terminal.rs`, `crates/cargo-tile/src/shim_registration/rows_readout.rs` — fixtures that build a group's ancestry, where the walk outcome changes them.
- `crates/cargo-tile/src/constants.rs` — `SUMMARY_CELL_TITLE`, the shell and wrapper lists, any new literal.
- `crates/cargo-tile/src/capture.rs` — item 18's title and its test (seat `impl`).
- `crates/cargo-tile/CHANGELOG.md` — the Changed line.
- `crates/tui_pane/src/tiles/draw.rs` — item 8's `CellContentExtent`, the provided `TileCells` method, the readout's colour, and its render test.
- `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — the export of `CellContentExtent`.
- `crates/tui_pane/CHANGELOG.md` — one Added line for `CellContentExtent`, and one Fixed line for item 14.
- `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/constants.rs` — item 14, with its render tests in `draw.rs`.

**Seats:** 2 writers — the Short-view naming in the census is disjoint from the drawing; everything drawn, `tui_pane` included, has one owner because items 14 to 16 join the table plan to the grid's row allocation.
- `impl` — `render.rs`, roster and fixture propagation, constants and changelogs, and all `tui_pane` grid, draw and export files; items 14 to 16.
- `test` — `census/command_text.rs` and `census/scan.rs`; publishes the naming and ancestry-walk APIs on the board before `impl` consumes them.

**Constraints from prior phases:** Phase 6 and 7: `ELISION` is cargo-tile's one mark; `AncestryLevel::AncestorAfterElision` and `mark_cut_line` (`render.rs` ~1157) are the ancestry cut mark. Phase 9: tables are planned before drawing (`ProcessTablePlan`), and a cut command fills its column before its mark. Phase 14 makes column offsets frame-wide through its staged grid, moves compiler and runs after command, insets every row one cell from the right frame, and drops columns before a command name wraps; the `state` slot in item 1 follows Phase 14's optional-column rule. Phase 12: content steps change moving heights, and `rows_drawn` and `rows_kept` come from one measurement (`content_row_use`); this phase's ancestry, naming and row-order changes keep that single source. Phase 13: retained rows protect a donor and painted rows decide a receiving step, so an optional ancestry row a command paints counts as an increase; `no_piece_rests_on_a_row_it_cannot_fill_while_a_mate_cuts_content` passes unchanged. New public items are only `CellContentExtent` and its provided `TileCells` method, both for item 8's foot. No new `#[allow]` or `#[expect]`. Ruling (showrunner, 2026-10-12): the cut mark's cell after `command` is always reserved, in a frame with no cut command too, so `compiler` stands two cells past the frame's longest command and no column moves when a command becomes cut or stops being cut. `compiler` follows the reserved command column by the normal column gap; `FrameTableColumnGrid::of` (`render.rs` ~2198) reserves the mark's cell only when `following_rows` holds, and `compiler_and_runs_follow_the_command_column` (~3754) asserts the single-row layout without it: reserve it always, change that expectation on purpose, and add a test comparing single-row, many-row, whole and cut frames with every column at one offset. Phase 14 fact: `content_row_use` is a `#[cfg(test)]` wrapper; production measures through `Cells::row_use`, `frame_content_row_use` and the immutable `FrameColumnGrids` (`render.rs` ~337, ~421, ~437, ~2133), so a change keeps that cache and both frame-wide grids. Current line references: `heading_gauge` ~3323, `ancestry_fit` ~1500, `ancestry_allocation` ~1564, `ancestry_levels` ~1625, `ancestry_lines` ~1734, `mark_cut_line` ~1285, `frame_summary_width` ~603 (was `summary_width`). Items 7 and 12 are one defect in `PathGroup::fitted_heading` (~1976): one repair, both examples tested. Phase 15: `crates/tui_pane/src/wrap.rs` has two rules. The public `wrapped` treats every span boundary as a word boundary and joins words with one unstyled space (`adjacent_spans_are_separate_words_joined_by_one_space`); cargo-tile's command cells pass a program and its arguments as two spans and depend on it. The crate-private `wrapped_runs` and `wrapped_preserving_whitespace` keep text that runs on across a style change as one word (`a_word_that_crosses_a_style_boundary_moves_whole_to_a_fresh_line`). So a part of a command drawn in its own style, item 17's profile mark included, is its own span only where a space belongs before it. A long name now breaks after a hyphen before it breaks inside a word, so ancestry rows are counted with the wrapper (`ancestry_rows`), never from text length. Any edit to `wrap.rs` also runs `test cargo-tile`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- Item 8, in the unit director's deep126x80 capture: every cell whose chain the walk cut opens with the inline mark and its foot count is in the cut colour; no such cell reads whole.
- Item 9, in the unit director's motion shots and the controlled workload's capture: no header or column shifts between frames from a value changing length, in the summary or in any cell. The one exception is the ruling's widening: a summed `mem` that first needs more cells than the column has widens it once; after that every offset holds and no column narrows.
- Item 10, in the unit director's shots: every hidden-rows mark has the ancestry marks' colour, measured from cell colours.
- Item 11, in the unit director's shots: at 40 and 48 columns and in the 40-column toast, the summary draws no blank row above its foot while a cell in its column hides an ancestry level.
- Item 12, in the unit director's shots: no heading begins with `…` where its path from `~` fits the same width.
- Item 13, in the unit director's shots: no row holds only `…` under a command that wrapped.
- Item 17, in the unit director's shots: a release run shows its profile in the summary row and in its cell, in Short view and in Long view, and in a row whose command is cut.
- Items 18 to 20, in the unit director's shots: the toast's title reads its new words at 200, 64 and 40 columns; at 40 columns a cut summary draws `cpu` and `mem` wherever its drawn rows hold them; no cell hides an ancestry level that the rows standing blank above its foot would hold.
- The shot set gains one shot of a long command cut at a cell's foot, filling its column before its `…` (phase 14's design check had no such shot).
- These earlier table tests pass unchanged: Phase 9's `a_table_that_yields_one_gap_yields_them_all_before_its_foot`, `a_fully_drawn_table_keeps_every_inter_group_gap` and `a_cut_command_fills_its_column_before_the_mark`; Phase 14's `stacked_command_cells_share_one_column_grid` and `columns_drop_before_a_command_name_wraps`.
- Unit director's captures, the eight shots (w200, w64, w48, w40, toast200, toast64, toast40, deep126x80): no two commands share a percent, one ancestry form, `cargo nextest run`'s inner cargo named with its subcommand, a Short-view name ending in ` …` exactly when it dropped arguments, a Short-view ancestry of one row naming the session runner by file name, no `/nix/store` path in Short view, and `┌ summary ─`.
- Phase 12's motion snap tests, Phase 13's `no_unchanged_cell_reverses_height_across_one_key`, and `painted_and_retained_rows_share_one_measurement` pass unchanged; the unit director's frame probe over toast64 and w200 in motion, and over the controlled workload that starts and stops projects on the key, reads 0 moving pieces with a blank row above their foot, and each key's last moving frame equals the settled frame after it, read through cargo-tile's `Cells`. Reversal and second-motion flags count only on keys with no content change in their window.
- A fresh helper's design check of those shots: pass.

### Phase 18 — Toast timing and group titles are named states  · status: todo

#### Work Order

**Goal:** Two public `tui_pane` APIs stop handing callers a bare `Option` for a domain state: a toast's timing reads as a named state, and a cell's group title reads as a named title. What the screen shows does not change. Three more toast states join them by the showrunner's placement (2026-10-12): a toast's action, toast focus, and a tracked item's start and finish.

**Spec:**
- **Toast timing.** `ToastView::linger_progress`, `ToastView::remaining_secs` and `TrackedItemView`'s public fields `linger_progress` and `elapsed` (`crates/tui_pane/src/toasts/item.rs` ~89; its only getter is `label()`) in `crates/tui_pane/src/toasts` hold named types, as do the renderer arguments fed from them, such as `spinner_style`'s: `ToastLingerState`, `ToastCountdownState` and `TrackedItemTiming`, or names like them. Each says what the absent case means. Any `Option` getter that stays exists only to convert at the boundary. Callers in cargo-port and cargo-tile move to the new types.
- **Group title.** `TileCells::group_title(&Id) -> Option<Span>` becomes a semantic title such as `TileGroupTitle::{Untitled, Titled(..)}`. Every implementor moves to it: cargo-tile and cargo-handler.
- **Toast action** (add-on, placed here by the showrunner, 2026-10-12). `ToastSpec.action` (`manager.rs` ~47), `Toast.action` (`toast.rs` ~141) and the public inputs in `commands.rs` (~138 and ~161) hold a bare `Option<Ctx::ToastAction>`, which hides that an available action takes a body row. They become a named type such as `ToastActivation<A>::{Informational, Opens(A)}` that states its row cost, and every caller in cargo-port converts.
- **Toast focus** (add-on, placed here by the showrunner, 2026-10-12). `StackLayout` in `crates/tui_pane/src/toasts/render/layout.rs` pairs `ToastPaneFocus` with `focused_toast_id: Option<ToastId>`, so a focused toast in an unfocused pane can be built. One type such as `ToastFocus::{PaneUnfocused, PaneFocusedWithoutToast, PaneFocusedOn(ToastId)}` replaces the pair, so the meaningless pair cannot be built.
- **Tracked item timing** (add-on, placed here by the showrunner, 2026-10-12). `TrackedItem`'s public fields `started_at` and `completed_at` and its getter `completed_at()` (`crates/tui_pane/src/toasts/item.rs`) are bare `Option<Instant>`, set and read by `crates/tui_pane/src/toasts/lifecycle.rs`, beneath the view's timing states above. The item's timing becomes one named state, the unknown-start behavior is kept, and no public field or getter of `TrackedItem` holds a bare `Option` for it.
- Apply the Type Design Contract and the user's style guide. No behavior change, no new `#[allow]` or `#[expect]`.

**Files:**
- `crates/tui_pane/src/toasts/view.rs`, `crates/tui_pane/src/toasts/item.rs`, `crates/tui_pane/src/toasts/toast.rs`, `crates/tui_pane/src/toasts/manager.rs`, `crates/tui_pane/src/toasts/mod.rs` — the named timing types and their getters.
- `crates/tui_pane/src/toasts/render/card.rs`, `crates/tui_pane/src/toasts/render/drawing.rs` — readers of the timing getters.
- `crates/tui_pane/src/tiles/draw.rs` — `TileCells::group_title` and its named title.
- `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` — the exports of the new public types.
- `crates/cargo-port/src/tui/app/mod.rs` — the toast timing caller.
- `crates/cargo-handler/src/render.rs` — the `group_title` implementor; any cargo-tile implementor or caller found by search moves too.
- `crates/tui_pane/src/toasts/commands.rs`, `crates/tui_pane/src/toasts/lifecycle.rs`, `crates/tui_pane/src/toasts/render/layout.rs` — the toast action's public inputs, the tracked item's timing writes and reads, and toast focus; cargo-port's callers of the toast action, found by search, convert too.
- `crates/tui_pane/CHANGELOG.md` — one Changed line.

**Seats:** 1 writer.

**Constraints from prior phases:** Phase 15 left the toast timing getters as they were, outside its items. Phase 15: a toast body is a list of `ToastRun`s, each with `RunBreaks::{AtSpaces, Never}`, wrapped by the crate-private `wrapped_runs`; the public `wrapped` treats every span boundary as a word boundary, and cargo-tile's tables depend on that, so this phase leaves `crates/tui_pane/src/wrap.rs` and the body's runs alone, and any edit to `wrap.rs` also runs `test cargo-tile`. Phase 16 kept `group_title`'s signature. These two changes are the public signature changes those phases deferred. The user approved them from the Phase 12 review.

**Acceptance gate:**
- No public getter or public field named above holds a bare `Option` for its domain state, and no named accessor hides one kept beneath it.
- A test covers each of toast focus's three states, and a test pins a tracked item whose start is unknown.
- lint and the tui_pane, cargo-tile, cargo-handler and cargo-port suites pass, with no test at a second or more.
- Rendered output is unchanged: the existing toast and tile render tests pass without edits to what they assert.

### Phase 19 — Slow tests start first  · status: todo

#### Work Order

**Goal:** On each machine, every non-reader test observed at 0.8 s or more in any measurement run starts before every faster non-reader test in a full `cargo nextest run`; the seven reader scenarios keep their existing priority and one-at-a-time rules. The tests item 5 names are also shortened to under a second, by the showrunner's placement (2026-10-12) on the user's standing rule that a test taking a second or more is a defect to fix; every other test is ordered only, not rewritten.

**Spec:**

1. **Measure before.** On each machine run the suite CI runs, `cargo nextest run --all-features --workspace --exclude cargo-mend --tests`, three times with JUnit turned on from outside the repo config: a file `junit.toml` holding `[profile.default.junit]` / `path = "junit.xml"`, passed as `--tool-config-file tile-fixes:<absolute path>/junit.toml`. Each `testcase` then carries its start `timestamp` and `time`. Every run writes the same `target/nextest/default/junit.xml`, so before the next run copy it to `<machine>-<before|after>-<n>.xml` (machine `natedev` or `mac`, `n` 1 to 3, or 4 to 6 for a repeated after batch) in the delegate session directory; the Mac lane copies each file there over `scp` as it finishes, so every run's evidence survives. Record the wall time of each run and, per test, its slowest time. On the Mac work in a clone outside `/tmp`, reached with `ssh mac`: `git clone git@github.com:natepiano/cargo-liner.git /Users/natemccoy/rust/cargo-liner-measure` if it is absent, then before every batch of runs `git -C /Users/natemccoy/rust/cargo-liner-measure fetch origin remove-running-section-tile-fixes`, `git -C … checkout --detach FETCH_HEAD`, and confirm `git rev-parse HEAD` equals the phase tip on natedev and `git status --short` is empty. (`/Users/natemccoy/rust/cargo-liner` is the user's checkout on `main`; leave it alone.)
2. **The slow set** is every test at 0.8 s or more in any of the six runs, less the seven reader scenarios the config already orders. Known members to confirm by name with `cargo nextest list`: in `cargo-tile::unit_tests`, `progress::capture::…::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`, eight in `shim_registration::wire::`, two in `hook::`; two each in `cargo-tile::shim_modes` and `cargo-tile::cli_lifecycle`; in `cargo-handler`, `census::codex::…::reads_threads_by_id_whoever_started_them` and `…::reads_interactive_threads_created_in_the_span`; in `tui_pane`, `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw`.
3. **One override.** Append to `.config/nextest.toml` one `[[profile.default.overrides]]` with `priority = 80` and a `filter` that names each slow test exactly: `binary_id(<id>) & (test(=<name>) | …)` per binary, joined with `|`. No regular expressions, so a test added later is not swept in by accident. A comment above it says what the list is, the threshold, the date measured, and how to re-measure. nextest resolves each setting from the first override that sets it, so the two existing priorities (100, 90), the `cargo-tile-readers` group and both `slow-timeout` entries keep working unchanged; the new override sets `priority` alone and names none of the reader scenarios.
4. **Measure after**, the same three runs per machine. A non-reader test that crosses 0.8 s in an after run and is not in the filter joins it, and the three after runs repeat once on both machines; a second newcomer is reported in the summary rather than chased.
5. **Shorten the named slow tests** (add-ons the showrunner placed here, 2026-10-12, with one more test the unit director saw at a second or more in phase 15's runs). Built first, before item 1's runs, so the six measurements see the shortened suite. Each test keeps what it asserts: none is weakened, split to hide its time, or marked ignored. Where a real spawned process sets the floor, the seat reports the measured floor in its summary and does not weaken the test to get under it.
   - `tui_pane` `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw` (0.65 to 0.88 s quiet, 2.15 s under load): under 0.3 s.
   - `tui_pane` `tiles::grid::tests::unchanged_mates_stay_between_their_endpoints_during_plus_and_minus_motion` (1.587 s in phase 15's run): under a second with the other suites running.
   - cargo-tile's reader scenarios: `reader_keeps_parent_family_across_its_only_childs_source_switch` (1.295 s; 1.856 s in phase 15's run) and every other reader scenario seen at a second or more. Each under a second with the other suites running.
   - cargo-tile's shim registration tests, each of which waits on a spawned process: `reader_scenarios::reader_attributes_compiler_cache_server_cpu_to_the_requesting_invocation` (1.158 s; 2.313 s in phase 15's run), `wire::darwin_birth_conversion_scopes_locale_and_keeps_cargo_environment` (1.136 s), `shared_capture::darwin_membership_beyond_the_credential_limit_keeps_account_children_running` (1.029 s), `wire::non_json_arguments_preserve_both_quiet_spellings` (1.026 s), `wire::darwin_birth_conversion_distinguishes_both_sides_of_dst_fallback` (1.003 s), `shared_capture::staged_executable_installs_toolchains_and_cleans_up` (2.366 s once), and in `shim_modes`: `extended_registration_is_world_readable_before_publication` (1.061 s), `unset_home_does_not_abort_either_capture_path` (1.196 s), `caller_descriptors_reach_captured_cargo` (1.512 s), `owner_repairs_parent_and_account_modes_without_touching_other_data` (1.497 s) and `reader_accepts_live_writer_under_different_locale_and_timezone` (2.134 s). Each under 0.5 s on a quiet machine and under a second at a load average of 42.

**Files:**
- `.config/nextest.toml` — one override and its comment. In scope for this unit by the showrunner's word (2026-10-08), though the Units row does not list it.
- `crates/tui_pane/src/attract/controller.rs`, `crates/tui_pane/src/tiles/grid.rs` — item 5's two `tui_pane` tests.
- `crates/cargo-tile/src/shim_registration/reader_scenarios.rs`, `crates/cargo-tile/src/shim_registration/wire.rs`, `crates/cargo-tile/src/shim_registration/shared_capture.rs`, `crates/cargo-tile/tests/shim_modes.rs` — item 5's cargo-tile tests and the test support they share.

**Seats:** 1 writer + 1 tester — one configuration owner and two measurement lanes on a byte-identical candidate.
- `impl` — owns `.config/nextest.toml`, publishes its `sha256sum` on the board after each edit, and runs every natedev before and after run; checks every exact filter name.
- `test` — no source file; copies the candidate config to the Mac clone (`scp` over `ssh mac` into `/Users/natemccoy/rust/cargo-liner-measure/.config/nextest.toml` after the checkout), verifies the same sha256 before each after-run batch, runs every Mac run, and hands `impl` the named JUnit files, the slow set and the start order. The before runs use the clean phase tip.
- Item 5 comes first, with two writers on disjoint files: `impl` shortens cargo-tile's tests and `test` shortens `tui_pane`'s two. Each runs lint once and its package's suite, then both take the roles above.

**Constraints from prior phases:** This phase runs after Phases 1 to 18, every phase planned to add tests or change shared grid code, so its timings measure the suite those phases ship. Phase 20 follows it by the showrunner's landing call (2026-10-12) and checks its own tests against this phase's line. Phases 1 to 18 added render tests to `tui_pane` and `cargo-tile`; each gate asked for under a second for its own tests, and this phase's line is 0.8 s, so the six measurements decide whether any of them joins the slow set. Seen over a second during Phase 8's runs under load: `tui_pane` `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw` (1.52 s) and several reader and `shim_modes` scenarios.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, after the config edit: green.
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, the source-switch reader is first, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- Item 5: `test tui_pane` and `test cargo-tile` pass; each named test runs under its stated time in the after runs on natedev and asserts what it asserted before; a test held up by a real spawned process is listed with its measured floor.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.

### Phase 20 — A cell that enters, leaves or settles moves on the clock  · status: todo

#### Work Order

**Goal:** In every frame of a key's motion on the controlled workload, a piece that enters or leaves slides, no piece changes height by more than one slide step in a frame, and the live capture counts 0 failures on each rule below. These are the motion cases Phase 14's live checks left open when the showrunner called its landing (2026-10-12: no fourth repair round; the open motion cases form a new last phase).

**Spec:** Evidence for every item: the round 3 build's live capture B, `results_p14.txt` sections `P14d.1` and `P14d.4` (copied into the delegate session directory before dispatch). Notation: start-end(h height, N content rows / R rows drawn, CUT = content hidden, IN = entering this frame); times are from the key.

1. **A cell that opens while another slides in starts at its opening height.** Known at Phase 14's checkpoint and worse than the Phase 13 build; Phase 14's last repair turn stopped without a repair. First `+` of a round, both rounds, at +0.009 s and +0.014 s, with a command's cell 0.10 s into its entry slide:
   `+0.000s 0-20(h20,N9/R19) 20-41(h21,N10/R20) 41-48(h7,N10/R6 CUT)`
   `+0.009s 0-12(h12,N9/R11) 12-31(h19,N10/R18) 31-43(h12,N10/R11 IN) 43-48(h5,N10/R4 CUT)`
   A cell appears at its end height in one frame, the summary drops 8 rows in that frame, and the cell that was sliding in falls from h7 to h5. On the Phase 13 build the same key brought one piece that entered at h2 and slid for 0.62 s. Required: the opening piece slides from its opening height; a piece still entering stays an entering piece and its height never falls; the residents move along the clock.
2. **A cell admitted into a running motion slides from its opening height.** Known at Phase 14's checkpoint and worse than the Phase 13 build, as item 1. Third `+` of a round, both rounds, at +0.205 s and +0.191 s:
   `+0.189s 0-17(h17,N15/R16) 17-22(h5 CUT) 22-30(h8 CUT) 30-39(h9 CUT) 39-48(h9 CUT)`
   `+0.205s 0-17(h17,N15/R16) 17-22(h5 CUT) 22-28(h6 CUT) 28-35(h7 CUT) 35-42(h7 CUT) 42-48(h6,N10/R5 CUT)`
   A command's cell enters at h6 in one frame and three cutting mates lose 2 rows each. On the Phase 13 build it entered at h2 and reached h5 0.71 s later.
3. **An empty cell slides out across a summary depth step.** 1 and 3 cells per round still lose their last 3 or 4 rows in one frame, each on a frame where the summary's depth steps (Phase 13 build: 4 and 3). Worst, second round, tenth key (`-`), +1.294 s, from h4:
   `+1.284s 0-11(h11,N12/R10 CUT) 11-16(h5,N0/R4) 16-20(h4,N0/R3) 20-29(h9 CUT) 29-38(h9 CUT) 38-48(h10 CUT IN)`
   `+1.294s 0-14(h14,N12/R13) 14-19(h5,N0/R4) 19-29(h10 CUT) 29-38(h9 CUT) 38-48(h10 CUT IN)`
   Required: a leaving piece is drawn until its slide ends, also across a depth step.
4. **The summary moves along the clock.** The summary changes height by 4 rows in one frame on 4 keys of the two rounds while its content does not change (the Phase 13 build does the same on 3). Required: no resident changes height by more than 3 rows in one frame.
5. **A rule for a cell that is itself sliding.** Every frame the resident spare-row count still binds (25 and 95 per round; Phase 13 build 267 and 186) is one of two forms: the cutting mate entered 0.10 s before the key and is still sliding in; or the holder's own content shrank less than 0.75 s earlier and it now slides down where it dropped in one frame before. The second form sets item 4's rule against the spare-row rule. Ruling (showrunner, 2026-10-12): a cell whose own content shrinks is a resident, so it gives its rows at once to a mate that cuts, with no slide; rows no mate wants follow the existing clock. Item 4's limit therefore excepts a resident giving rows to a cutting mate, and the first form (a mate still sliding in) is an entering piece under Phase 14's ruling. The capture tool's classes follow this.
6. **One motion per key, read on keys free of arrivals.** The reversal and one-motion gates could not be judged: no key of the workload was free of a command arriving in its window, and arrivals aimed at the second and third keys land 0.19 to 0.22 s after them (0.02 to 0.05 s before them on the round 2 build; cause not determined). Find the cause of the shift, give the workload keys with no arrival, then require 0 reversals and one motion per key on those.
7. **The checks Phase 14 did not run:** the changing-width capture for band escapes, and the frame probe (`cap6`) repeated on a quiet machine.
8. **A staying cell keeps its rows on a key that frees room.** Phase 14's design check on its checkpoint build, `motion_09_minus` line 22 and `motion_10_minus` lines 26 and 31: a cell that stays shows two process rows before and after the key (`motion_08_minus`, `motion_11_minus`) and one row with `…` 0.33 s into it, while the summary already stands at its full 16 rows with four blank rows between projects. A piece is leaving in both shots, but the pieces cut are staying ones. One frame per key cannot tell a dip inside the key from a settled six-cell layout, so read it on the live capture first. Required: on `-`, a staying cell keeps the rows it had, and the summary grows only by rows the leaving cell has given up.

Diagnosis from Phase 14's last repair turn, for items 1 and 2: `open_waiting_commands_in_current_motion` changes the slots while it keeps the running transition's source and clock, and seeds the new piece's previous height to 1; `drawing_at` then draws the new piece at that shared clock's position. A piece admitted into a running motion has no admission time of its own. Two fixtures built from the live heights did not reproduce the entry-at-height frame, so the first step is to instrument the admission path on the live workload and record the transition's source, destination and clock at the frame of each case. The design question: what owns an admitted piece's origin and remaining slide time, and how do the dividers move without lowering a piece that is still entering.

Each repaired item has a render-path regression that fails without it, built from the live roles (resident, entering and how far into its slide, leaving; opening queued or cell claimed), and its fixture joins `every_frame_of_a_key_keeps_the_motion_rules`.

**Pending decision: whether a job cell that changes columns is laid out once; and whether a `-` pressed during a motion starts at once**

These are two add-ons that change what the user sees, so each answer is the user's and arrives through the showrunner (showrunner, 2026-10-12). The answers, for each one: build it in this phase, put it on the next-items list, or drop it. This phase does not dispatch while the block stands.

Actual problem:
1. A job cell's heading and ancestry are its name, and its two pieces lay content out separately (`crates/tui_pane/src/tiles`: `draw_placements`, `TileCells::draw`; cargo-tile's `Cells`), so a row shows twice or drops out while the cell changes columns, in a closing column too.
2. A `-` pressed during a motion starts 0.64 to 0.93 s late with nothing changing on screen (`TileGrid`'s step queue). Phase 13's third repair round found the cause: a queued key waits in `pending` until the preceding transition's 720 ms timer ends, even after its geometry has reached its end.

What exists now:
- First: Found in Phase 3. Shots `P/ph3s/shots/ph3s_on_12_close_motion.png` and `ph3s_off_09_close_later.png`; capture `P/ph3s/off/cap.json` key 13 at 1.2 s.
- Second: Found in Phases 2 and 13. Shot `P/ph2d/off/ph2d_off_08_close_motion.png`; the live check saw a `-` start 0.55 s late, 0.19 s after the previous key's last visible change.

What should change:
- First: Content is laid out once and the two pieces show consecutive rows; a render test with a job cell crossing in both directions shows each content row at most once per frame. If built, Files gains cargo-tile's `Cells` in `crates/cargo-tile/src/render.rs`.
- Second: The step starts within one frame of the previous motion's last visible change, or the wait is visible.

Recommendation:
1. Build it in this phase: the doubled or missing row shows in normal use during a motion, and this phase owns the motion code.
2. Build it in this phase: a key that answers more than half a second late is a defect against Fast, the cause is known, and it lives in this phase's file.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/constants.rs` — the motion code and its tests.
- `crates/tui_pane/CHANGELOG.md` — one Fixed line.

**Seats:** 1 writer + 1 tester — the items sit in one motion path, so one seat writes.
- `impl` — `grid.rs`, `draw.rs`, `tiles/constants.rs`, the `tui_pane` changelog, and `.config/nextest.toml` only when a new test reaches 0.8 s.
- `test` — no source file; before any repair it replays the live cases, writes each item's fixture roles and expected heights for `impl`, then runs the live capture tools on each candidate build.

**Constraints from prior phases:** Phase 14's ruling holds: a piece that is entering or leaving slides, and the spare-row rule binds cells that are neither, and every settled frame. Everything Phase 14's live check met stays met: the emptied cell on the seventh key stands 0 frames above its floor, the summary shows 0 blank rows, no empty cell leaves from its full floor height, one piece per `+`, 0 band escapes. No new row probe or row measurement per moving frame; the seven-cell moving budget stays 29, and speed stays within 1 ms of the Phase 13 build. Three repair rounds in Phase 14 each fixed their fixture and moved the defect to a neighbouring live state (memory `attempts-tile-phase14-motion-rows`): a repair is proved on the live capture before it closes. This phase runs after Phase 19 by the showrunner's landing call; a test it adds that reaches 0.8 s joins Phase 19's override. This phase runs after Phase 17, which changes the same admission and queuing path (its item 14), the summary's separators and row order: replay every case on the build Phase 17 leaves before repairing, and keep Phase 17's key-joining regressions and shared measurements. Items 5 and 8 may already be partly met (`draw.rs` tests ~2522 and ~2535 cover immediate summary donation): repair only what a replayed live failure shows. The immediate transfer item 5 permits is one event for every piece it touches: the resident that gives rows and each mate that takes them may change by more than 3 rows in that frame; the Goal, the tests and the capture tool's counters class it the same way, and the jump gate excepts it. Type design: `HeightSnap::previous_height -> Option<u16>` (`grid.rs` ~367) folds disabled probing into missing history, and `open_waiting_commands_in_current_motion` (~2128) stands a new piece in at height 1; the repair names admission time and height history as states at that boundary, with no bare `Option` or height sentinel as the answer. The shared fixture is `grid.rs::tests::motion_rules::every_frame_of_a_key_keeps_the_motion_rules`. A new test reaching 0.8 s joins Phase 19's exact-name filter in `.config/nextest.toml`, which `impl` then edits, keeping Phase 19's reader priorities and timeout rules.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `verify.sh test` for `tui_pane`, `cargo-tile`, `cargo-handler` and `cargo-port`: green, no test at a second or more.
- Live, controlled workload, two rounds, read with the Phase 14 tools: command cells at once 0; pieces vanishing from over 2 rows 0; resident jumps over 3 rows 0 outside the immediate transfer item 5 permits; binding spare-row frames 0 under item 5's ruling; on keys free of arrivals 0 reversals and one motion per key; band escapes 0 on the changing-width capture.
- Speed on a quiet machine: moving and settled medians within 1 ms of the Phase 13 build.
- Shots of the motion keys and a design check by a fresh helper.
