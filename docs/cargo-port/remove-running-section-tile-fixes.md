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
  - `crates/cargo-tile/src/wrap.rs` — `Wrap::push` (72), `wrapped` (118), `split_at_cells` (151), tests from 160.
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

#### Work Order

**Goal:** A command cut at a cell's foot shows as much of itself as its column holds before the mark, and a cell that hides rows behind the mark leaves no blank row above its foot.

**Spec:**

Evidence (Phase 8's design check after its second repair round; moved here by the hard landing rule; shots `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, text beside each in its `.txt`). Reproduce each case in a failing buffer test before changing anything.

The shot line numbers came from a moving workload; the fixtures named in each item are the reproduction.

1. **A cut command fills its column** (cargo-tile `render.rs`). The wrapper breaks at a word, and Phase 8's mark (`mark_table_continuation`) follows the last whole word, so a cut command leaves empty cells before its `…`: at 64, 48 and 40 columns, `cargo…` and `nextest…` stand with 2 to 6 free cells before the mark. `TableContinuation::CommandWasCut` is a unit variant and the mark rebuilds only cells already in the buffer, so it cannot add the omitted characters: give the variant a semantic payload (for example `CommandWasCut { continuation: CutCommandContinuation }`, carrying the styled text that did not fit) rather than a bare `Option<T>`. Rule: when a command's wrapped lines are cut at the foot, the last drawn line holds the value's next characters up to one cell short of the command column's width, then the mark (`cargo next…`, `nextest r…`), so the mark sits at the column's far edge. A whole value followed by omitted rows keeps Phase 8's far-edge mark with its free cells, and an exactly fitting whole value keeps Phase 8's wrap onto a free row. Test: `a_cut_command_fills_its_column_before_the_mark` (an 11-cell command column, `cargo nextest run`, one row for it: `cargo next…`).
2. **A cell hiding rows leaves no blank row above its foot** (cargo-tile `render.rs`). In the deep 126 by 80 capture, cells given six rows and wanting fourteen draw one process row with the mark and then two blank rows above the foot. Cause as read: `PathGroupDraw::height_with_gap` always counts the one-row gap after a group, so the next group's heading plus first process row (two rows) cannot fit in the two blanks. Rule: at a vertical cut the inter-group gap yields: the next group in order is drawn without the gap before it when that lets its heading and first process row fit; content is never skipped out of order. Keep drawn group height and the trailing gap as separate values rather than an `Option<T>`. The mark moves to the last process line drawn. A heading is still admitted only with its first process row (Phase 8). Test: `a_cell_hiding_rows_leaves_no_blank_row_above_its_foot` — three groups of one one-line process each in a five-row table: the first two fill it with no gap between them, the third stays hidden, and the mark sits on the second group's process row with no blank row below it.
3. **A command refused for height stays in the summary** (cargo-tile `render.rs`, test only). `a_command_refused_for_height_opens_when_there_is_room` in `tui_pane` checks only placement counts. Add `a_command_refused_for_height_remains_in_the_summary_until_it_opens` through the real `Cells`: in a short area the refused command is listed in the summary and has no cell; after a resize to a tall area its cell opens.

These states last one frame and show only as TUI text, so buffer tests and the captures are their whole surface.

Changelog: one Changed line under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/cargo-tile/src/render.rs` — both items and their tests.
- `crates/cargo-tile/CHANGELOG.md` — the Changed line.

**Seats:** 1 writer — both items live in cargo-tile's table drawing in one file.
- `impl` — `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/CHANGELOG.md`; runs the one lint and the four suites.

**Constraints from prior phases:** Phase 6 made headings and ancestry whole or marked with the one glyph `ELISION`. Phase 8 made every table cut end in the mark (`mark_cut_line`, `mark_last_drawn_table_line`, `TableContinuation`: a value cut inside itself carries an attached mark, a whole value followed by omitted rows carries it at the command column's far edge, and one that exactly fills its column wraps its last word onto a free row first), admits a directory heading only with its first process row, drops an empty `compiler` or `runs` column, and set cargo-tile's cell floor to `MIN_CELL_HEIGHT` (6). Phase 8's mark tests pass unchanged except where item 1 moves the mark of a cut value, each named in the summary with its old and new text. No new public item. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test this phase adds or changes takes a second (older slow tests are Phase 13's).
- Unit director's settled captures beside real cargo commands, Phase 8's set (64, 48, 40 and 200 columns by 50 rows, 126 by 80 deep, the toast at 200, 64 and 40): every cut command's mark at its column's far edge, and no blank row above a foot while the mark says rows are hidden.
- A fresh helper's design check of those shots: pass.

### Phase 10 — The summary takes the rows it asks for, not a second position  · status: todo

#### Work Order

**Goal:** The summary takes a further position in its column only when it would use more than half of it, and no cell is given a row past its ask while a cell in its column is short, so no rows stand blank in any cell while another in its column hides rows.

**Spec:**

Evidence (Phase 2's design check, 200x50, the tenth command cell opening while the summary asks 12 content rows): the left column settles at 20, 13 and 12 rows where it was 16, 15 and 14; the summary shows seven blank rows; every other column holds four cells of 12 rows.

Cause as read; reproduce it in a failing test before changing anything. `summary_depth` (`grid.rs:1932`) deepens the summary as soon as `summary_share` (1912) at the current depth is under what it asks. `summary_share` calls `shares` (1824), which gives every piece its base share by weight (`apportion`, the summary's weight is its depth) and then serves the summary only from `spare`, the rows its column-mates do not ask for. In a 47-row column of four positions the base share is 11 or 12 rows and the mates want theirs, so a summary asking 13 is one or two rows short. It then takes a whole second position, 23 rows, and gives back only what its two remaining mates ask: 20, 13, 12, with seven rows nobody in that column wants, while the cell it pushed out crowds another column.

Rule:
1. Let `slot` be the summary column's framed share for one position at the current depth, from the same model `shares` now uses (`apportion_shared_run`, with the shared border restored as `summary_share` does), not raw `apportion`. When the summary is short by `slot / 2` rows or fewer, it stays at this depth and takes the shortfall from its mates, the tallest first, one row at a time, never taking a mate under `min_tile_height`. The floor is judged on each mate's final framed rectangle, after `share_borders`, not on its raw share: a piece that is not at the bottom may hold `min_tile_height - 1` raw rows because the shared border supplies its last one (Phase 8's shared-run rule). Name the half as a constant in `crates/tui_pane/src/tiles/constants.rs`.
2. When it is short by more than that, or its mates cannot give the rows without going under the minimum, `summary_depth` deepens as today.
3. `summary_depth` stays a non-decreasing function of what the summary asks, with everything else fixed, so a summary growing a row at a time never flips between depths and back.
4. `queue_with_depth` and the motion code do not change: a depth change is still queued as Phase 2 left it.

Leftover rows go to a cell that is hiding rows (showrunner, 2026-10-09). Evidence: Phase 9's shots at `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, each `.png` with a `.txt` of its exact text: the widget-enhancements command cell holds one directory and one process yet draws a blank row above its foot (w64 L20, w200 L21), while a cell in the same column hides rows behind `…` (pputb_tree, w200 L32 and w64 L32; tool-based-ui-geometry-material, w200 L37). Cause as read; reproduce it in a failing test first: `shares` hands rows beyond what every piece asks to pieces by weight, so a piece can be given rows past its own ask while a mate in its column is still short of its ask. Rule 5: within a column, no piece receives a row beyond its ask while a mate in that column is short of its ask; such rows go to the short mates, the one shortest of its ask first, one row at a time. Only rows that no piece in the column asks for may stand blank, and they go as today. This holds for the summary's mates under rules 1 to 3 as well, and never takes a piece under `min_tile_height`.

Tests, all pure, on settled grids:
- `grid.rs`: `a_summary_one_row_short_takes_it_from_its_mates` — the evidence case: eleven positions, the summary at depth 1 with exactly the rows it asks, three mates sharing the rest, none under the minimum.
- `grid.rs`: `a_summary_far_short_still_takes_a_second_position` — a summary asking a slot and three quarters.
- `grid.rs`: `summary_depth_never_falls_as_the_summary_asks_for_more` — every demand from 1 row to the column's height, at 6, 9, 10 and 13 command cells.
- `grid.rs`: `no_mate_goes_under_the_minimum_to_feed_the_summary` — a bottom mate and a mate that is not at the bottom, each measured framed.
- `grid.rs`: `no_piece_gets_a_row_past_its_ask_while_a_column_mate_is_short` — rule 5: a column of three command pieces, one asking far less than its weight would give and one asking more than it can have: the short piece receives every row the small one does not ask for.
- `draw.rs`: `the_summary_shows_no_blank_row_while_a_cell_in_its_column_is_short` — through `draw_tile_grid` with `StubCells` demands: when any cell in the summary's column has fewer body rows than it asks, the summary has no body row beyond what it asks.
- `draw.rs`: `a_cell_draws_no_spare_row_while_a_column_mate_hides_rows` — rule 5 through `draw_tile_grid` with `StubCells` demands, command cells only.
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

**Constraints from prior phases:** Phase 2 made `TileGrid::drawing_at(area, growth, raw)` the pure entry to a motion and made `queue_with_depth` keep every surviving cell in a column it starts or ends the queue in; neither changes here. Phase 3 made a column the step removes close as one piece (`BandPieceMotion::ClosingWithColumn`; test `no_piece_in_a_closing_column_changes_height`, with and without a widened summary over the closing column) and gave each piece a name role (`TilePiece`, `PieceName`, `piece_name`); a change to the summary's rows keeps both, and the `a_title_is_on_screen_once_*` and `an_empty_cell_number_is_on_screen_once_*` render tests pass unchanged. Phase 5 added `TileGrid::set_min_tile_width` and the crate-private `holds_in`, which checks every arrangement still to be drawn, each at its own depth: a summary that stays shallower lets a grid hold in a smaller window, it must never let a wider arrangement still in flight be drawn under the floor, and `holds_in`'s tests pass unchanged. `cargo-handler` draws on the same grid, so its summary follows the same rule. Phase 5 also made the grid answer the summary alone itself (`drawing_at` returns one summary piece when `holds_in` fails), so a grid that does not hold never reaches `shares`. Phase 8 made the height floor true: every piece `shares` returns is at least `min_tile_height` rows once framed, at exact `shared_run` boundaries too, and cargo-tile's floor is `MIN_CELL_HEIGHT` (6) through `TileGrid::set_min_tile_height`; rule 1's "never under `min_tile_height`" rests on that, and Phase 8's floor tests pass unchanged. The `grid.rs` line references above were read before Phase 8: find each function by name (after Phase 8, `shares` is near 1832, `summary_share` near 1925, `summary_depth` near 1946). Phase 8 also made a table cut at its foot end in the mark, the summary included (hidden later rows too); a summary given fewer or more rows here keeps that mark, and Phase 8's mark tests pass unchanged. No new public item. No new `#[allow]` or `#[expect]`: the summary lists each one this phase adds or moves, with its file and line.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test this phase adds or changes takes a second (older slow tests are Phase 13's).
- Unit director's capture at 200x50 with seven `+` and seven `-` (it reaches ten cells and four columns), transparent off: in every settled frame the summary shows no blank body row while a cell in its column is short of what it asks, and Phase 2's motion measures still read zero.
- Unit director's eight shots (w200, w64, w48, w40, toast200, toast64, toast40, deep126x80): no cell draws a blank row above its foot while a cell in its column hides rows behind `…`.
- A fresh helper's design check of the settled shots and one motion shot per step, against Phase 2's shots of the same steps: pass.

### Phase 11 — One column grid, and every row clear of the right frame  · status: todo

#### Work Order

**Goal:** In every frame, a column starts at the same offset in every command cell, compiler and runs stand right after command, every row keeps one cleared cell before the right frame, and a cell labels only the columns its drawn rows fill.

**Spec:**

Evidence: shots in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, each `.png` with a `.txt` of its exact text. Shot lines come from a moving workload; the fixture named in each item is the reproduction. Reproduce each item in a failing buffer test (render, then read the buffer back) before changing anything. Line numbers are HEAD `3ce40d59`; Phase 9 edits `render.rs` first, so find each function by name.

1. **One column grid per frame** (`crates/cargo-tile/src/render.rs`: `TableLayout::of` 1687, `fitted_constraints` 2074, `table_column_spacing` 2116, `tile_demands` 353, `table_height` 492). Today each cell fits its own widths and gaps, so `mem` starts at col 37 in one w200 cell and col 39 in another, and w64 cells differ in `parent` width and gap (w64 L21, L27, L38). Rule: column widths, the gap and the longest command are fitted once per frame over every row of every command cell, then laid out per cell width. Two command cells of the same inner width draw the same columns at the same offsets, and a column has the same width in every command cell that draws it. `tile_demands` measures with the same grid that `draw` uses, so demand and draw agree. The summary keeps one grid of its own (its column set differs by `SUMMARY_HIDDEN_COLUMNS`) under the same rules. Test: `stacked_command_cells_share_one_column_grid` — two groups whose pid, cpu and compiler values differ in width, drawn as two cells of one width through `Cells`: every header starts at the same x in both.
2. **compiler and runs right after command** (`fitted_constraints` 2074: `command` is the one `Constraint::Min` and takes all slack, so `compiler` and `runs` sit at the right frame, ~130 cells from their data in w200 L24–L37 and deep L21–L47). Rule: `command` is as wide as the frame's longest command line (`longest_command_width`, `command_line_width`), capped by what the cell leaves; `compiler` and `runs` follow it after one gap; slack stands after the last column. Test: `compiler_and_runs_follow_the_command_column` — at 200 columns, `compiler` starts one gap after the longest command's last cell.
3. **One cleared cell before the right frame** (`indented` 2597 insets only the left; `draw_path_group` 1978 and `heading_gauge` 2478 use the whole `area.width`). Rows run flush into the frame: `99.8%│` (w64 L3), `…│` (w200 L15), `command│` (w40 L24). Rule: every content row of the summary and of every command cell leaves its last interior column blank, matching the left: column headers, headings and their gauge, table rows, ancestry rows, and Phase 8/9 cut marks, whose "far edge" becomes one cell in. Demands are measured at the same inset width. Name the inset in `crates/cargo-tile/src/constants.rs`. Test: `no_content_row_touches_the_right_frame` — summary and one command cell at 40, 64 and 200 columns with a gauge heading, a wrapped command and a cut mark: the last interior column is blank on every row above the foot.
4. **Columns from the rows actually drawn** (`visible_columns` 2367 decides from every row, so a `runs` header stands over an empty column when its only value is in a row hidden behind `…`: w200 L30–L32, toast64 L122–L125). Rule: an optional column (`state`, `compiler`, `runs`) keeps its slot in the frame grid when any command row in the frame holds a value; a cell draws its header and values only when one of its drawn rows holds a value, and otherwise leaves the slot blank. Positions never move for it. Test: `a_column_whose_only_value_is_hidden_draws_no_header` — a cell whose only `runs` value is in the row the cut hides: no `runs` header in that cell.
5. **One drop rule, before the command name wraps** (`TableLayout::of` 1697–1712 drops `TABLE_COLUMN_DROP_ORDER` columns only until the constraints fit, so commands wrap to `cargo` / `clippy` while `start` and `dur` stay: w40 L4–L5, w48 L56–L57 and L72–L73). Rule, for summary and command cells alike through one function: while a row's command name (program plus its Short-view name, `named()`) would wrap, drop the next column in `TABLE_COLUMN_DROP_ORDER`; argument lists beyond the name still wrap down the column. Test: `columns_drop_before_a_command_name_wraps` — `cargo clippy` in a summary and a command cell at inner widths 38 and 46: one line each, with the same columns gone from both.

Changelog: one Changed line under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/cargo-tile/src/render.rs` — the frame grid, column placement, right inset, column visibility, drop rule, and their tests.
- `crates/cargo-tile/src/constants.rs` — the right inset.
- `crates/cargo-tile/CHANGELOG.md` — the Changed line.

**Seats:** 1 writer — every item is cargo-tile's table layout in one file, and its tests live in that file's test module.
- `impl` — `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/CHANGELOG.md`; runs the one lint and the four suites.

**Constraints from prior phases:** Phase 5 set `MIN_CELL_WIDTH = 40`, and the summary is alone at inner widths up to 37; every header, value and foot is whole, marked with `ELISION`, or absent. Phase 6 added `table_column_spacing` and `command_line_width`. Phase 8 added `TableContinuation`, `mark_table_continuation` and `mark_cut_line`, admits a heading only with its first process row, and made `visible_columns` drop an all-`None` compiler and an all-zero runs column. Phase 9 gives `TableContinuation::CommandWasCut` a payload, puts a cut command's mark at its column's far edge, and lets the inter-group gap yield at a vertical cut; items 2 and 3 move that far edge, and Phase 9's mark tests change only where it moves, each named in the summary with old and new text. Phase 10 does not touch cargo-tile. No new public item. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- Unit director's captures beside real cargo commands, the eight shots (w200, w64, w48, w40, toast200, toast64, toast40, deep126x80): shared columns line up across stacked cells, compiler and runs stand after command, no row touches the right frame, no header stands over an empty column, and no command name wraps while a droppable column is drawn.
- A fresh helper's design check of those shots: pass.

### Phase 12 — One reading per command, readable inks, and a foot that fades  · status: todo

#### Work Order

**Goal:** Each command shows its own progress once, ancestry marks a cut one way, the Short view keeps the subcommand, the summary title has its blank cell, every ink on the grid meets its contrast floor, a fading cell's foot fades with it, and a word that fits a line is never split.

**Spec:**

Evidence: shots in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, text beside each in its `.txt`. Contrast is measured from cell colours, never PNG pixels (Phase 7 gotcha). Reproduce each item in a failing test before changing anything. `render.rs` line numbers are HEAD `3ce40d59`; find functions by name.

Text:
1. **Each command shows its own progress** (`render.rs` `heading_gauge` 2478 draws the first `Working` row's reading on the directory heading). In the summary, two `cargo check` runs in obsidian_knife (deep L12–L14) both read 84%, while one's cell reads 58% (deep L76). In a command cell, the second heading `[hana-linux-1] unavailable` repeats the lead's 99.8% gauge (deep L22, L26). Rule: every percent on screen belongs to one command, and each command with a reading shows its own. A heading carries a gauge only when one command in its group has a reading. When two or more do, each such row carries its own `percent_reading` in its `state` cell, which joins for that case (author's call). A group whose rows read only through an enclosing capture already shown in the cell (`RowProvenance::Enclosing`) draws no gauge. Tests: `two_commands_in_one_directory_show_their_own_progress`; `a_heading_does_not_repeat_its_cells_progress`.
2. **One ancestry form** (`render.rs` `ancestry_levels` 1231, `ancestry_fit` 1165, `ancestry_lines` 1340). Today one row draws `… <nearest>` inline and two or more draw a separate `…` row (deep L42, L52, L62, L72). The inline `…` sits in an unstyled span, so it draws white (#ffffff) on a dimmed row (#616163). Rule: at every budget, a cut is marked inline at the head of the first level drawn after it (`… <pid> <command>`), and the root is kept when the budget is two or more. `AncestryLevel::Elided` goes. The mark takes the same faded ink as that level's command text. Tests: `an_ancestry_cut_is_marked_inline_at_every_budget` (budgets 1 to 4 over a six-level chain), `the_ancestry_mark_takes_its_rows_faded_ink`. Phase 7's ancestry-level tests change only where the form changes, each named in the summary with old and new text.
3. **Short view keeps the subcommand after a flag** (`crates/cargo-tile/src/census/command_text.rs` `named` 115 stops at the first flag via `names_the_command` 191, so nextest's inner `cargo --… run` draws as a bare `cargo`: w200 L32, deep L37, L48). Rule: `named()` skips flags, and the value of a flag in `SUMMARY_HIDDEN_VALUED_FLAGS`, and keeps the words that name the command after them. Dropped arguments are not marked here: that is a held user decision, not this phase's. Test: `the_short_name_keeps_a_subcommand_after_a_flag` (`--color always nextest run` → `nextest run`; `--locked check` → `check`).
4. **One blank cell after the summary title** (`crates/cargo-tile/src/constants.rs:166` `SUMMARY_CELL_TITLE = " summary"`, written verbatim by `GridLines::add_titled`, `crates/tui_pane/src/pane/frame.rs:481`; every shot L1 reads `┌ summary───`). Rule: one blank cell before and after the title, as the sccache label and toast title have. cargo-handler's own summary title is out of this phase (author's call: the approved evidence is cargo-tile's). Test: `the_summary_title_has_a_blank_cell_after_it` — the top row reads `┌ summary ─`.
5. **The toast reads in plain words** (`crates/cargo-tile/src/capture.rs:53–56`). Today: "In front of cargo for …; \`cargo tile uninstall\` gives cargo its name back.", with literal backticks, and the command breaks across lines at 200 and 40 (toast40 L91–L96). Rule: the body says what the user gets and how to undo it in plain words, with no shim mechanics and no backticks. The toolchain name stays. `cargo tile uninstall` is drawn in a highlight ink and never breaks across lines. Test: `the_capture_toast_names_its_undo_command_on_one_line` at card widths 36 and 58.
6. **The toast's check mark draws** (`crates/tui_pane/src/toasts/render/card.rs:89` writes U+2713; it shows as an empty box in the shots). Rule: the mark draws as a glyph in the user's terminal font (check on the Mac). If it draws there, the fix is the capture renderer's font fallback (the unit director's tooling, no repository change). If it does not, use a glyph that font has.

Inks (built-in themes Default Dark and Default Light, transparent off; grounds are the ones `tui_pane` paints, `pane_background(true)` and `pane_background(false)`, `crates/tui_pane/src/pane/chrome.rs`; shots read #0e0e11 and #2e2e39 dark, #e0e0e9 light summary):
7. **Pid identity colours** (`crates/cargo-tile/src/theme/mod.rs` `FAMILY_COLORS` 25, `family_color` 105: one solarized set for both appearances; light 2.44 to 3.47:1, dark magenta 2.95:1 on the summary grounds). Rule: every colour `family_color` can hand out measures 4.5:1 or more on both grounds of the active theme, and the clearance and more-than-one-family tests still pass. One set cannot meet it in both appearances (it would need relative luminance ≥ 0.30 on #2e2e39 and ≤ 0.13 on #e0e0e9), so the set follows the appearance. Test: `every_family_colour_meets_contrast_on_both_grounds_in_every_built_in_theme`.
8. **The red count in the foot readout** (`crates/tui_pane/src/tiles/draw.rs` `rows_readout_lines` 511 writes the count in `error_color()`; Default Dark `error = Red` (`crates/cargo-tile/src/theme/builtins.rs:86`) draws #cd0000: 3.30:1 on #0e0e11, 2.30:1 on #2e2e39). Rule: the over-budget count, and the `@ W` width written in the same ink, measure 4.5:1 or more on both grounds in every built-in theme of cargo-tile, and of cargo-handler, which draws the same readout (`crates/cargo-handler/src/theme.rs:370`). Test: `the_readout_count_meets_contrast_on_both_grounds_in_every_built_in_theme`, one in each app.
9. **Light summary green** (Default Light `success = Rgb(0, 120, 0)`, `builtins.rs:136`: 4.33:1 on #e0e0e9; the shot read 4.05:1 from pixels). Rule: every ink a table row draws at fade 0 (`text_default`, `label_color`, `success_color`, `warning_color`, `secondary_text_color`) measures 4.5:1 or more on both grounds in every built-in theme. Test: `table_inks_meet_contrast_on_both_grounds_in_every_built_in_theme`, next to `status_line_text_meets_contrast_in_every_built_in_theme` (builtins.rs 436, whose `contrast_ratio` it reuses).
10. **Dimmed finished rows** (`crates/cargo-tile/src/roster.rs` `faded_at` 137 carries a finished row toward its ground evenly over `DEFAULT_FADE_SECONDS`; the summary's #607374 on #2e2e39 is that blend about half way, 2.95:1). Rule (author's call; the fade stays an exit): a finished row's ink stays at 3:1 or more on its ground until half of the fade has run, then reaches the ground at the end; summary and cells alike. Test: `a_finished_row_stays_readable_through_the_first_half_of_its_fade`.

Fade:
11. **A fading cell's foot fades with it** (`draw.rs` `draw_cell` 387 → `draw_rows_readout` 474 writes full-strength inks; deep L59–L69 show a near-black cell with a bright `content rows: 10  r/c: 9/124`). Rule: every span of the foot readout, and of the summary foot, is drawn as `blend_color(ink, ground, fade)` with the cell's own fade and ground, the same fade its contents use. `TileCells` gains one provided method returning a cell's fade, default none, so cargo-handler and `draw_tile_cell` keep today's drawing. cargo-tile answers it from the one function `draw_group` uses (`heading_fade(&rows).min(group.lead.faded())`, render.rs 791). Tests: `draw.rs` `a_fading_cells_readout_fades_with_it` through `draw_tile_grid` with `StubCells`; `render.rs` `a_finished_commands_cell_reports_the_fade_its_contents_use`.

Wrapping (showrunner, 2026-10-09):
12. **A word moves whole to a fresh line when it fits there** (`crates/tui_pane/src/wrap.rs`, Phase 7's wrapper). toast64 and toast200 L45–L46 split the toolchain name at a hyphen, `nightly-2026-09-03-x86_64-unknown-` / `linux-gnu.`, though the whole 44-cell word fits on the card's 56-cell line. Rule: a word that does not fit in what is left of a line holding text moves whole to the next line whenever it fits on a line by itself; only a word longer than a whole line is broken, at a hyphen if it has one, else at the width. Test: `a_hyphenated_word_moves_whole_when_it_fits_a_fresh_line` — the toolchain name after a short sentence at width 56 lands whole on the second line; a word longer than the width still breaks at its last hyphen that fits.
13. **A toast keeps one cleared row above and below** (`crates/tui_pane/src/toasts/render/card.rs` clears `TOAST_SIDE_GAP` cells left and right only, ~146–175; `crates/tui_pane/src/toasts/render/layout.rs` places the card). Phase 9's design check: the card's bottom border (toast64 L48) sits on the row directly above the frame's bottom border (L49), and its top border sits directly under text (toast64 L44 under the heading at L43; toast40 L40 under the column header). Rule: the card is placed one row above the frame's bottom border, and one cleared row is drawn above its top border, matching the side gap; a stack of toasts keeps the same one-row gap between cards. Name the gap in `crates/tui_pane/src/constants.rs` beside `TOAST_SIDE_GAP`. Test: `a_toast_keeps_one_cleared_row_above_and_below` — a card drawn over filled text: the rows directly above and below it are blank across the card's width plus its side gaps, and the frame border is untouched.

Changelogs: one Changed line each under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md` and `crates/tui_pane/CHANGELOG.md`, and in `crates/cargo-handler/CHANGELOG.md` if its theme changes.

**Files:**
- `crates/cargo-tile/src/render.rs` — progress per command, ancestry form, the cell fade answer, tests.
- `crates/cargo-tile/src/census/command_text.rs` — `named`, its test.
- `crates/cargo-tile/src/capture.rs` — toast body and its test.
- `crates/cargo-tile/src/roster.rs` — fade curve and its test.
- `crates/cargo-tile/src/constants.rs` — `SUMMARY_CELL_TITLE`, any new literal.
- `crates/cargo-tile/src/theme/mod.rs`, `crates/cargo-tile/src/theme/builtins.rs`, `crates/cargo-tile/themes/*.toml` — family colours per appearance, inks, contrast tests.
- `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/*.toml` — readout red, its test.
- `crates/tui_pane/src/tiles/draw.rs` — the fade method, faded foot, render test.
- `crates/tui_pane/src/toasts/render/card.rs` — the toast's cleared rows and its test; the mark glyph only if the terminal font lacks U+2713.
- `crates/tui_pane/src/toasts/render/layout.rs`, `crates/tui_pane/src/constants.rs` — the toast's cleared rows.
- `crates/tui_pane/src/wrap.rs` — whole-word move to a fresh line, its test.
- `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md` — Changed lines.

**Seats:** 2 writers — the text and fade work is cargo-tile drawing code; the ink work is theme data and contrast tests, with the shared readout in `tui_pane`.
- `impl` — items 1 to 5, 10 and cargo-tile's half of 11: `render.rs`, `command_text.rs`, `capture.rs`, `roster.rs`, cargo-tile `CHANGELOG.md`; hub: `crates/cargo-tile/src/constants.rs` (both writers name literals there).
- `test` — opens as impl; items 6 to 9, 12, 13 and tui_pane's half of 11: `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/toasts/render/card.rs`, `crates/tui_pane/src/toasts/render/layout.rs`, `crates/tui_pane/src/constants.rs`, `crates/tui_pane/src/wrap.rs`, `crates/cargo-tile/src/theme/`, `crates/cargo-tile/themes/`, `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/`, the tui_pane and cargo-handler changelogs. Lands the `TileCells` method first and tells `impl` its name.

**Constraints from prior phases:** Phase 4: no ink changed then; any ink change now is checked on both themes, transparent on and off. `theme/state.rs` keeps its four `#[expect(clippy::expect_used)]`. Phase 6 and 7: `ELISION` is cargo-tile's one mark. `AncestryLevel::AncestorAfterElision` and `mark_ancestry_cut` are the ancestry cut mark, `status_line_text_meets_contrast_in_every_built_in_theme` holds status inks at `MIN_STATUS_LINE_TEXT_CONTRAST` = 4.5, and named colours resolve through its xterm table. Phase 7's `#[expect(clippy::expect_used, reason = "tests should panic on unexpected values")]` in `crates/tui_pane/src/wrap.rs` is user-approved. Phase 8: toasts keep one cleared cell each side (`TOAST_SIDE_GAP`), and `toast_card_width` decides card width. Phase 11 makes column offsets frame-wide, moves compiler and runs after command, and insets every row one cell from the right frame; the `state` slot in item 1 follows Phase 11's optional-column rule. No existing `tui_pane` public signature changes; the one addition is the provided `TileCells` method. cargo-handler's cell sizes do not change. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane`, `... lint cargo-tile` and `... lint cargo-handler`, each once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- Unit director's captures, the eight shots (w200, w64, w48, w40, toast200, toast64, toast40, deep126x80), transparent off on both themes for the ink items: no two commands share a percent, one ancestry form, `cargo nextest run`'s inner cargo named with its subcommand, `┌ summary ─`, the toast in plain words with its command and toolchain name each on one line and a cleared row above and below it, and every measured ink at its floor from cell colours.
- A fresh helper's design check of those shots: pass.

### Phase 13 — Slow tests start first  · status: todo

#### Work Order

**Goal:** On each machine, every non-reader test observed at 0.8 s or more in any measurement run starts before every faster non-reader test in a full `cargo nextest run`; the seven reader scenarios keep their existing priority and one-at-a-time rules. Ordering only: no test is rewritten (user's words: "for now").

**Spec:**

1. **Measure before.** On each machine run the suite CI runs, `cargo nextest run --all-features --workspace --exclude cargo-mend --tests`, three times with JUnit turned on from outside the repo config: a file `junit.toml` holding `[profile.default.junit]` / `path = "junit.xml"`, passed as `--tool-config-file tile-fixes:<path>/junit.toml`. Each `testcase` then carries its start `timestamp` and `time`. Record the wall time of each run and, per test, its slowest time. On the Mac work in a clone outside `/tmp`, reached with `ssh mac`: `git clone git@github.com:natepiano/cargo-liner.git /Users/natemccoy/rust/cargo-liner-measure` if it is absent, then before every batch of runs `git -C /Users/natemccoy/rust/cargo-liner-measure fetch origin remove-running-section-tile-fixes`, `git -C … checkout --detach FETCH_HEAD`, and confirm `git rev-parse HEAD` equals the phase tip on natedev and `git status --short` is empty. (`/Users/natemccoy/rust/cargo-liner` is the user's checkout on `main`; leave it alone.)
2. **The slow set** is every test at 0.8 s or more in any of the six runs, less the seven reader scenarios the config already orders. Known members to confirm by name with `cargo nextest list`: in `cargo-tile::unit_tests`, `progress::capture::…::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`, eight in `shim_registration::wire::`, two in `hook::`; two each in `cargo-tile::shim_modes` and `cargo-tile::cli_lifecycle`; in `cargo-handler`, `census::codex::…::reads_threads_by_id_whoever_started_them` and `…::reads_interactive_threads_created_in_the_span`; in `tui_pane`, `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw`.
3. **One override.** Append to `.config/nextest.toml` one `[[profile.default.overrides]]` with `priority = 80` and a `filter` that names each slow test exactly: `binary_id(<id>) & (test(=<name>) | …)` per binary, joined with `|`. No regular expressions, so a test added later is not swept in by accident. A comment above it says what the list is, the threshold, the date measured, and how to re-measure. nextest resolves each setting from the first override that sets it, so the two existing priorities (100, 90), the `cargo-tile-readers` group and both `slow-timeout` entries keep working unchanged; the new override sets `priority` alone and names none of the reader scenarios.
4. **Measure after**, the same three runs per machine. A non-reader test that crosses 0.8 s in an after run and is not in the filter joins it, and the three after runs repeat once on both machines; a second newcomer is reported in the summary rather than chased.

**Files:**
- `.config/nextest.toml` — one override and its comment. In scope for this unit by the showrunner's word (2026-10-08), though the Units row does not list it.

**Seats:** 1 writer + 1 tester — one configuration owner and two measurement lanes on a byte-identical candidate.
- `impl` — owns `.config/nextest.toml`, publishes its `sha256sum` on the board after each edit, and runs every natedev before and after run; checks every exact filter name.
- `test` — no source file; copies the candidate config to the Mac clone (`scp` over `ssh mac` into `/Users/natemccoy/rust/cargo-liner-measure/.config/nextest.toml` after the checkout), verifies the same sha256 before each after-run batch, runs every Mac run, and hands `impl` the JUnit files, the slow set and the start order. The before runs use the clean phase tip.

**Constraints from prior phases:** This phase runs last, after every phase that adds tests or changes shared grid code, so its timings measure the suite the run ships. Phases 1 to 12 added render tests to `tui_pane` and `cargo-tile`; each gate asked for under a second for its own tests, and this phase's line is 0.8 s, so the six measurements decide whether any of them joins the slow set. Seen over a second during Phase 8's runs under load: `tui_pane` `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw` (1.52 s) and several reader and `shim_modes` scenarios.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, after the config edit: green.
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, the source-switch reader is first, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.
