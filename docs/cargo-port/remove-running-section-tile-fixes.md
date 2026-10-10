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

#### Work Order

**Goal:** The grid shows the command cells only when the summary shows at least as many of its rows there as it would alone, the choice holds steady through one-row changes and motions, and a `view` setting (`auto`, `summary`, `cells`) picks that rule, the summary alone, or today's rule, in cargo-tile and cargo-handler alike.

**Spec:**

Evidence (user, via the showrunner, 2026-10-09): on the Mac at about 77×40 with `c36fd503`, the summary sits at depth 1 top left reading `content rows: 50` while it shows about 8 of them, beside five command cells in two columns at the 40-wide floor. It had shown the summary alone, then switched back to cells when a job left. Cause: `GridDisplay` (`crates/tui_pane/src/tiles/grid.rs` `display` ~1213) picks `SummaryAlone` only when `holds_in` fails, that is, when the command cells cannot fit at their floor; what the summary hides never counts. Reproduce that case in a failing test before changing anything.

1. **The collapse rule (`auto`)** (`grid.rs` `display`, `holds_in`). Rule: the cells view is shown only when the summary shows at least as many content rows in it as it would alone, measured at the allocation the grid settles on after the summary has deepened and taken spare rows as far as Phase 10's rules let it (not the rows of a frame mid-motion). Otherwise the summary is shown alone. Both sides are rows the summary draws, not rows it is given: measure the cells candidate (the summary's settled piece) and the summary-alone candidate (the whole area) through `TileCells::rows_drawn`, since a table given fewer rows yields its gaps (Phase 10). Today `draw_tile_grid` (`crates/tui_pane/src/tiles/draw.rs` ~170) asks `display()` before `measure_row_steps`, and `display` (`grid.rs` ~1354) knows only `holds_in` (~1332): measure both candidates in the same frame's probe, then commit one held `GridDisplay` that drawing, hit-testing and focus movement all read. Command cells never draw under their floor in any view. Test also `a_demand_change_in_one_frame_decides_the_view_from_that_frames_rows`. Test: `a_summary_hiding_rows_beside_cells_shows_alone` — the screenshot's case: 77×40, a summary asking 50 content rows, five command cells: `SummaryAlone`.
2. **The choice holds steady.** The view changes only on a settled grid, as one state change in a settled frame, not a motion: the summary alone bypasses `GridMotion` today (`summary_alone_drawing`) and keeps doing so. During a motion the grid keeps the view it held when the motion began. Switching from the summary alone back to cells needs the cells view to show the summary's rows with one demand step (3 rows, named in `crates/tui_pane/src/tiles/constants.rs`) to spare, or its whole ask; switching from cells to the summary alone needs only the rule to fail. So a one-row change in the summary's ask never flips it back and forth. Hold the decision in the grid as a named state, never a bare bool. Tests: `a_one_row_change_in_the_summarys_ask_does_not_flip_the_view` (ask moves by one row each way across the switch point: the view changes once); `the_view_holds_through_a_motion`.
3. **A view setting** (showrunner, 2026-10-09). One Stepper row (`< value >`, the kind the appearance steppers use), framework-owned like `tiles.fill`: `tiles.view` with `auto` (default; items 1 and 2), `summary` (always the summary alone) and `cells` (today's rule: cells whenever they fit at the floor, the summary alone only when they don't). `TileView` lives beside `TileFill` (`crates/tui_pane/src/app_config/tile_fill.rs`) in a new `crates/tui_pane/src/app_config/tile_view.rs`, spelled in `config.toml` as `auto`, `summary`, `cells`. `FrameworkSetting` gains `TileView`, the framework config trait gains `tile_view_mut` (`crates/tui_pane/src/app_settings/step.rs`), and the settings rows gain a `tile_view` push beside `tile_fill` (`app_settings/rows.rs`). `TileSettings` (`tiles/settings.rs`, private) carries the view inside the grid; the apps hand it over through a new public `TileGrid::set_view(TileView)`, beside `set_min_tile_width`, called each frame before `draw_tile_grid` (cargo-tile `render.rs` ~291, cargo-handler `render.rs` ~89), as `TileGrowth` is passed today. cargo-tile and cargo-handler each store it in their `[tiles]` table, push the row beside `fill`, and write it in their default config text. Tests: `the_view_row_steps_through_auto_summary_and_cells`; `the_view_row_shows_the_file_spelling`; `summary_view_shows_the_summary_alone_while_cells_fit`; `cells_view_shows_cells_whenever_they_fit_at_the_floor`; `no_view_draws_a_cell_under_its_floor`; in each app, a config round trip of `view = "summary"`.

Changelogs: one Added line and one Changed line under `## [Unreleased]` in `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` and `crates/cargo-handler/CHANGELOG.md`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — the collapse rule, the held view, `set_view`, their tests.
- `crates/tui_pane/src/tiles/draw.rs` — measuring both candidates before the view is committed.
- `crates/tui_pane/src/tiles/settings.rs`, `crates/tui_pane/src/tiles/constants.rs` — the view in `TileSettings`, the switch margin.
- `crates/cargo-tile/src/render.rs`, `crates/cargo-handler/src/render.rs` — `set_view` from config each frame.
- `crates/tui_pane/src/app_config/tile_view.rs`, `crates/tui_pane/src/app_config/mod.rs`, `crates/tui_pane/src/lib.rs` — `TileView` and its export.
- `crates/tui_pane/src/app_settings/step.rs`, `crates/tui_pane/src/app_settings/rows.rs` — the stepper row and its tests.
- `crates/cargo-tile/src/config.rs`, `crates/cargo-tile/src/settings.rs` — cargo-tile's `[tiles]` view and its row.
- `crates/cargo-handler/src/config.rs`, `crates/cargo-handler/src/settings.rs` — cargo-handler's `[tiles]` view and its row.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md` — Added and Changed lines.

**Seats:** 2 writers — the rule and setting in `tui_pane`, and each app's config wiring.
- `impl` — items 1, 2 and 3's framework half: every `crates/tui_pane/` path above (`draw.rs` included) and the tui_pane changelog. Lands `TileView`, `TileGrid::set_view`, `FrameworkSetting::TileView`, `tile_view_mut` and the `tile_view` row push first and posts their names on the board.
- `test` — opens as impl; item 3's app half: `crates/cargo-tile/src/config.rs`, `crates/cargo-tile/src/settings.rs`, `crates/cargo-tile/src/render.rs` (the `set_view` call only), `crates/cargo-handler/src/config.rs`, `crates/cargo-handler/src/settings.rs`, `crates/cargo-handler/src/render.rs`, the cargo-tile and cargo-handler changelogs. Writes each app's round-trip test first, then wires the field on `impl`'s posted names.

**Constraints from prior phases:** Phase 5 set the cell floor (`MIN_CELL_WIDTH = 40`) and the summary alone below it; that floor still holds in every view. Phase 10 apportions rows by `CellRowDemand { exact_framed, stepped_framed }`: depth and weights read stepped, spare rows read exact, and an exact-only change updates a settled allocation without a motion; this phase compares exact rows. Phase 10's repair also has each cell report the rows it actually draws at a height (provided `TileCells::rows_drawn`, cargo-tile answering from `process_table_plan`) and moves rows a cell cannot use to a mate whose next content step they complete; the rows the summary shows in the cells view are those it draws, not those it is given. Phase 12 snaps moving heights in the cells view only and leaves `display` alone. The queue and transition machinery (motion code) does not change. `display` stays shared by drawing, hit-testing and focus movement. New public items are only `TileView`, `TileGrid::set_view`, and the row push and trait method. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test this phase adds or changes takes a second (older slow tests are Phase 17's).
- Unit director's captures at 77×40 with a summary asking 50 rows and five command cells: `auto` shows the summary alone; `cells` shows cells; `summary` shows the summary alone at 200×50 too; plus the eight shots, no view drawing a cell under its floor.
- A fresh helper's design check of those shots: pass.

### Phase 12 — A moving piece is drawn only at heights its content fills  · status: todo

#### Work Order

**Goal:** Through every motion, each moving piece stands at a height its content fills, so no piece shows a blank row above its foot mid-motion while its content hides rows, and settled layouts and Phase 9's look are unchanged.

**Spec:**

Evidence (Phase 10's design check and a live frame probe, 2026-10-09): settled layouts obey Phase 10's rules (0 blank rows in 3645 settled pieces), but 195 of 569 moving pieces showed blank rows above their foot. toast64 at 3 s, in motion: the summary needs 18 rows, holds 14 body rows, its gaps between directories give way so it draws 13, and one row stands blank above its foot `mem 12.2G` while two cells in its column hide rows. Measured causes: (1) on a turn where content and height move together, the content grows first and the old height is drawn for about 0.73 s, so every gap gives way and two rows stand blank; (2) eased heights mid-motion (`lerp`, `eased`, `band_piece_dividers` in `crates/tui_pane/src/tiles/grid.rs`) land inside a content step: a directory's heading and first process row come in together, and Phase 9 yields every gap at once, so a height one row short of a step draws a blank row. Reproduce both in failing tests before changing anything.

Rule (showrunner, 2026-10-09, option A): in the cells view, the grid draws a moving piece only at a height its content fills. A height is valid for a piece when `TileCells::rows_drawn`, asked for `draw::content_area(frame_inner(piece_rect))` (the area after frame and readout, not the framed rect), answers that area's full height, or when the content is whole there (rows nobody asks for may still stand blank, as settled). Solve each band's dividers together, not one at a time: from the interpolated dividers, pick the nearest set of dividers where every resident piece stands at a valid height, the pieces still tile the band with shared lines, each resident piece keeps `min_tile_height`, and no piece's height reverses direction within a step; both ends of a motion are unchanged. Entering and leaving pieces keep collapsing below `min_tile_height` as they do today, and `ClosingWithColumn` is untouched; snap only what stays valid for them. A view switch (Phase 11) is a settled-frame state change, not a motion, so nothing here animates it.

Cost: `drawing_at` cannot call `TileCells`, so measure in two passes on one clock per frame: pass one computes the interpolated dividers, pass two probes only the heights a bounded directional search around each interpolated divider needs (at most a content step, about three rows, each way), never every height the 1,000-point progress scale crosses. Rename Phase 10's cache types to names that state their lifetime (`ContentRowMeasurement` -> `FrameContentRowUsage`, `CellRowMeasurements` -> `FrameCellRowUsage`) and keep one probe per cell, width and height per frame. Settled frames take the path they take today. Performance gate: a frame of the toast64 case in motion probes `rows_drawn` no more than twice the cells in view times the search width; pin it in a test that counts calls.

Tests (pure, through `drawing_at` at fixed progress, and rendered):
- `grid.rs`: `a_moving_piece_snaps_to_a_height_its_content_fills` — a piece whose content steps by two rows, sampled at every progress value: each drawn height is one its `rows_drawn` answers.
- `grid.rs`: `a_snapped_height_moves_one_way_through_a_motion` — the same motion: no piece's height reverses, and both ends equal the settled layouts.
- `grid.rs`: `a_column_still_tiles_its_band_while_pieces_snap` — every frame: pieces' framed rects share their lines and fill the band.
- `grid.rs`: `two_stepped_mates_both_stand_at_heights_they_fill` — two adjacent pieces whose contents step by two rows compete for the same rows: both fill, both keep their floors, the band is tiled.
- `grid.rs`: `arriving_and_leaving_pieces_snap_without_a_floor` — an arrival and a departure through a step; `ClosingWithColumn` unchanged.
- `grid.rs`: `a_moving_frame_probes_a_bounded_number_of_heights` — the call-count gate above.
- cargo-tile `render.rs` buffer test: `a_moving_summary_draws_no_blank_row_above_its_foot` — the toast64 case: a summary needing 18 rows, mates hiding rows, a motion sampled through its turn: no frame shows a blank row above the summary's foot.
- `a_turn_that_grows_content_and_height_draws_no_blank_row` — the Together-turn case through cargo-tile's buffer path.
- Phase 2's motion tests, Phase 3's `no_piece_in_a_closing_column_changes_height`, and Phase 10's settled tests pass unchanged.

Changelogs: one Fixed line under `## [Unreleased]` in `crates/tui_pane/CHANGELOG.md` and `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — the band divider solver in motion, the renamed per-frame usage cache, tests.
- `crates/tui_pane/src/tiles/draw.rs` — the second probe pass through `TileCells::rows_drawn`.
- `crates/cargo-tile/src/render.rs` — the two buffer tests.
- `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — Fixed line.

**Seats:** 1 writer + 1 tester — the snap is one hand in `grid.rs`; the cargo-tile buffer tests are written from this Spec.
- `impl` — `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs`, both changelogs.
- `test` — `crates/cargo-tile/src/render.rs`: both buffer tests, failing until `impl` lands the snap.

**Constraints from prior phases:** Phase 2 made `TileGrid::drawing_at(area, growth, raw)` the pure entry to a motion, every divider interpolated once and shared by both pieces it bounds; Phase 2's motion measures read zero and still do. Phase 3 closes a removed column as one piece (`BandPieceMotion::ClosingWithColumn`), which does not change height. Phase 8 set the height floor (`MIN_CELL_HEIGHT` 6 in cargo-tile). Phase 9 plans each table before drawing (`ProcessTablePlan`): all gaps fit or all give way, and a heading comes only with its first process row; this phase keeps that and moves the piece's height instead. Phase 10 added the provided `TileCells::rows_drawn(&self, content, area) -> u16` (default `area.height`, so cargo-handler draws as today), `measure_row_steps` and its per-frame cache applied to motion destinations only, and step-aware transfers in settled layouts (`spare_rows`, `next_useful_step`, `move_useful_step`); settled behavior does not change here. Phase 11 changes `display` and the held view only, and a view switch is a settled-frame state change outside `GridMotion`; snap only the cells view's geometry. No new public item. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test this phase adds or changes takes a second.
- Unit director's frame probe over the same workload as Phase 10's (toast64 and w200 through their motions): 0 moving pieces with a blank row above their foot while their content hides rows; settled count stays 0.
- The nine shots plus one motion shot per step: a fresh helper's design check, pass.
- No frame of the 200x50 seven-`+`-seven-`-` capture is slower than before this phase by more than 1 ms.

### Phase 13 — One column grid, and every row clear of the right frame  · status: todo

#### Work Order

**Goal:** In every frame, a column starts at the same offset in every command cell, compiler and runs stand right after command, every row keeps one cleared cell before the right frame, and a cell labels only the columns its drawn rows fill.

**Spec:**

Evidence: shots in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, each `.png` with a `.txt` of its exact text. Shot lines come from a moving workload; the fixture named in each item is the reproduction. Reproduce each item in a failing buffer test (render, then read the buffer back) before changing anything. Line numbers are HEAD `3ce40d59`; Phase 9 edits `render.rs` first, so find each function by name.

1. **One column grid per frame** (`crates/cargo-tile/src/render.rs`: `TableLayout::of` ~1775, `fitted_constraints` ~2320, `table_column_spacing` ~2362, `tile_demands` ~413, `table_height` ~552). Today each cell fits its own widths and gaps, so `mem` starts at col 37 in one w200 cell and col 39 in another, and w64 cells differ in `parent` width and gap (w64 L21, L27, L38). Rule: column widths, the gap and the longest command are fitted once per frame over every row of every command cell, then laid out per cell width. Two command cells of the same inner width draw the same columns at the same offsets, and a column has the same width in every command cell that draws it. `tile_demands` measures with the same grid that `draw` uses, so demand and draw agree. The summary keeps one grid of its own (its column set differs by `SUMMARY_HIDDEN_COLUMNS`) under the same rules. Build it in stages, each a named type, because today's `DrawnRow` exists only after a cell's `TableLayout` does and so cannot feed the grid that makes it: `ProcessRowValues` (a row's cell texts, before any width) → `FrameCommandColumnGrid` (widths, gap and longest command, fitted once over the `ProcessRowValues` of every row of every command cell in the frame; the summary's grid is fitted over the summary rows alone) → `CellTableLayout` (that grid laid out at one cell's inner width) → `LaidOutProcessRow` (a row wrapped to its layout, borrowing the command text only for an actual cut, as `DrawnRow` does) → `ProcessTablePlan`. `Cells` builds the frame's two grids once per frame, and its `demands` and `draw` both read those same grids. Test: `stacked_command_cells_share_one_column_grid` — two groups whose pid, cpu and compiler values differ in width, drawn as two cells of one width through `Cells`: every header starts at the same x in both.
2. **compiler and runs right after command** (`fitted_constraints` ~2320: `command` is the one `Constraint::Min` and takes all slack, so `compiler` and `runs` sit at the right frame, ~130 cells from their data in w200 L24–L37 and deep L21–L47). Rule: `command` is as wide as the frame's longest command line (`longest_command_width`, `command_line_width`), capped by what the cell leaves; `compiler` and `runs` follow it after one gap; slack stands after the last column. Test: `compiler_and_runs_follow_the_command_column` — at 200 columns, `compiler` starts one gap after the longest command's last cell.
3. **One cleared cell before the right frame** (`indented` ~2906 insets only the left; `draw_path_group` ~2230 and `heading_gauge` ~2787 use the whole `area.width`). Rows run flush into the frame: `99.8%│` (w64 L3), `…│` (w200 L15), `command│` (w40 L24). Rule: every content row of the summary and of every command cell leaves its last interior column blank, matching the left: column headers, headings and their gauge, table rows, ancestry rows, and Phase 8/9 cut marks, whose "far edge" becomes one cell in. Demands are measured at the same inset width. Name the inset in `crates/cargo-tile/src/constants.rs`. Test: `no_content_row_touches_the_right_frame` — summary and one command cell at 40, 64 and 200 columns with a gauge heading, a wrapped command and a cut mark: the last interior column is blank on every row above the foot.
4. **Columns from the rows actually drawn** (`visible_columns` ~2676 decides from every row, so a `runs` header stands over an empty column when its only value is in a row hidden behind `…`: w200 L30–L32, toast64 L122–L125). Rule: an optional column (`state`, `compiler`, `runs`) keeps its slot in the frame grid when any command row in the frame holds a value; a cell draws its header and values only when one of its drawn rows holds a value, and otherwise leaves the slot blank. Positions never move for it. Today `draw_process_table` draws the header before it plans the rows, and `ProcessTablePlan` records only a group count and the gap policy, so plan the viewport first: the plan names the rows that receive at least one visible line, it is computed before the header, and the header reads it. Slot presence (the frame grid's, from every command row in the frame) and header-and-value visibility (each cell's, from its drawn rows) are two separate fields, never one flag. Test: `a_column_whose_only_value_is_hidden_draws_no_header` — a cell whose only `runs` value is in the row the cut hides: no `runs` header in that cell.
5. **One drop rule, before the command name wraps** (`TableLayout::of` ~1785–1800 drops `TABLE_COLUMN_DROP_ORDER` columns only until the constraints fit, so commands wrap to `cargo` / `clippy` while `start` and `dur` stay: w40 L4–L5, w48 L56–L57 and L72–L73). Rule, for summary and command cells alike through one function: while a row's command name (program plus its Short-view name, `named()`) would wrap, drop the next column in `TABLE_COLUMN_DROP_ORDER`; argument lists beyond the name still wrap down the column. Test: `columns_drop_before_a_command_name_wraps` — `cargo clippy` in a summary and a command cell at inner widths 38 and 46: one line each, with the same columns gone from both.
6. **A whole command never carries the hidden-rows mark after its last letter** (`mark_table_continuation`, `mark_cut_line`: when the last drawn row's command is whole and rows are hidden below it, the `…` lands right after its last letter, `cargo check…` at w40 L14 and toast40 L14, which reads as a cut command). Rule: the hidden-rows mark of a whole command stands at the command column's far edge (one cell in, item 3), the same place as a cut command's mark, with at least one blank cell between it and the command's last letter. Test: `a_whole_command_keeps_a_blank_cell_before_the_hidden_rows_mark` — a cell at inner width 38 hiding rows below a whole `cargo check`: the row reads `cargo check`, then blanks, then `…` at the column's far edge.

Changelog: one Changed line under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/cargo-tile/src/render.rs` — the frame grid, column placement, right inset, column visibility, drop rule, the whole command's mark, and their tests.
- `crates/cargo-tile/src/constants.rs` — the right inset.
- `crates/cargo-tile/CHANGELOG.md` — the Changed line.

**Seats:** 1 writer — every item is cargo-tile's table layout in one file, and its tests live in that file's test module.
- `impl` — `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/CHANGELOG.md`; runs the one lint and the four suites.

**Constraints from prior phases:** Phase 5 set `MIN_CELL_WIDTH = 40`, and the summary is alone at inner widths up to 37; every header, value and foot is whole, marked with `ELISION`, or absent. Phase 6 added `table_column_spacing` and `command_line_width`. Phase 8 added `TableContinuation`, `mark_table_continuation` and `mark_cut_line`, admits a heading only with its first process row, and made `visible_columns` drop an all-`None` compiler and an all-zero runs column. Phase 9 gives `TableContinuation::CommandWasCut` a `CutCommandContinuation` payload built only for the cut row (`DrawnRow::cut_command_continuation`), puts a cut command's mark at its column's far edge, and plans each table before drawing (`ProcessTablePlan` from `process_table_plan`): a table keeps every gap between directories when all of them fit, and otherwise yields every gap, with spare rows once above the foot. Item 1's staged pipeline ends in that plan and keeps it; `a_table_that_yields_one_gap_yields_them_all_before_its_foot` and `a_fully_drawn_table_keeps_every_inter_group_gap` pass unchanged. Items 2 and 3 move that far edge, and Phase 9's mark tests change only where it moves, each named in the summary with old and new text. Phase 10's repair has `Cells` report the rows its table draws at a given height, read from `process_table_plan`; item 1's pipeline keeps that answer and reads it from the same grids. Phase 12 snaps each moving piece's height to one its content fills, measured through that answer; item 1's pipeline serves those measurements too. The frame grid owns only frame-wide inputs: each column's intrinsic width, which optional slots are present, and the longest command; each cell's `CellTableLayout`, built at that cell's inner width, derives its own drops and spacing from them, since cells in one frame differ in width. One immutable frame snapshot serves `tile_demands`, drawing, `rows_drawn` and Phase 12's motion probes, so all four read the same layout. Extend `ProcessTablePlan::rows_drawn` with the identities of the visible rows (item 4 needs them) rather than adding a second row-count path. Phase 10's `a_freed_row_goes_to_a_mate_whose_next_step_it_completes` and `a_freed_row_stays_when_no_mates_next_step_fits` pass unchanged. Phase 11 touches it only in `config.rs` and `settings.rs`. No new public item. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- Unit director's captures beside real cargo commands, the eight shots (w200, w64, w48, w40, toast200, toast64, toast40, deep126x80): shared columns line up across stacked cells, compiler and runs stand after command, no row touches the right frame, no header stands over an empty column, no command name wraps while a droppable column is drawn, and no whole command has `…` right after its last letter.
- Phase 12's motion tests pass unchanged, and the unit director's frame probe over toast64 and w200 in motion reads 0 moving pieces with a blank row above their foot while their content hides rows.
- A fresh helper's design check of those shots: pass.

### Phase 14 — One reading per command, one ancestry form, and a titled summary  · status: todo

#### Work Order

**Goal:** Each command shows its own progress once, ancestry marks a cut one way, the Short view keeps the subcommand, marks dropped arguments and names one session runner by file name, and the summary title has its blank cell.

**Spec:**

Evidence: shots in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, text beside each in its `.txt`. Reproduce each item in a failing test before changing anything. `render.rs` line numbers are HEAD `3ce40d59`; find functions by name.

1. **Each command shows its own progress** (`render.rs` `heading_gauge` ~2787 draws the first `Working` row's reading on the directory heading). In the summary, two `cargo check` runs in obsidian_knife (deep L12–L14) both read 84%, while one's cell reads 58% (deep L76). In a command cell, the second heading `[hana-linux-1] unavailable` repeats the lead's 99.8% gauge (deep L22, L26). Rule: every percent on screen belongs to one command, and each command with a reading shows its own. A heading carries a gauge only when one command in its group has a reading. When two or more do, each such row carries its own `percent_reading` in its `state` cell, which joins for that case (author's call). A group whose rows read only through an enclosing capture already shown in the cell (`RowProvenance::Enclosing`) draws no gauge. Tests: `two_commands_in_one_directory_show_their_own_progress`; `a_heading_does_not_repeat_its_cells_progress`.
2. **One ancestry form** (`render.rs` `ancestry_levels` ~1341, `ancestry_fit` ~1260, `ancestry_lines` ~1450). Today one row draws `… <nearest>` inline and two or more draw a separate `…` row (deep L42, L52, L62, L72). The inline `…` sits in an unstyled span, so it draws white (#ffffff) on a dimmed row (#616163). Rule: at every budget, a cut is marked inline at the head of the first level drawn after it (`… <pid> <command>`), and the root is kept when the budget is two or more. `AncestryLevel::Elided` goes. The mark takes the same faded ink as that level's command text. Tests: `an_ancestry_cut_is_marked_inline_at_every_budget` (budgets 1 to 4 over a six-level chain), `the_ancestry_mark_takes_its_rows_faded_ink`. Phase 7's ancestry-level tests change only where the form changes, each named in the summary with old and new text. After item 5 a cut happens mostly in Long view, but Short view's fallback (no qualifying session runner, drawn as today) can still cut, so this one form applies in both views. **A cut ancestry gives up its gap row** (`ancestry_budget` always subtracts `ANCESTRY_GAP_HEIGHT`). Today a cell whose ancestry hides levels still keeps the blank row between ancestry and table: deep126x80 L24, L41, L49, L58, L66, L74; toast200 L38 (phase 10 design check). With a budget of exactly one row, nothing above the table is drawn, so a one-process group with ancestors grows only in steps of two. Rule: when the ancestry hides any level, the gap row gives way and carries one more level, like the gaps between directories; a whole ancestry keeps its gap. Test: `a_cut_ancestry_gives_its_gap_row_to_one_more_level` — a six-level chain at budgets 1 to 3: no blank row between the last level and the `pid` header.
3. **Short view keeps the subcommand after a flag** (`crates/cargo-tile/src/census/command_text.rs` `named` 115 stops at the first flag via `names_the_command` 191, so nextest's inner `cargo --… run` draws as a bare `cargo`: w200 L32, deep L37, L48). Rule: `named()` skips flags, and the value of a flag in `SUMMARY_HIDDEN_VALUED_FLAGS`, and keeps the words that name the command after them. When the Short view drops any argument, the name ends in ` …`, the same mark the ancestry rows use (`cargo check …`); with nothing dropped it ends at the name (`cargo check`). The user decided this, 2026-10-09 (via the showrunner). Phase 13's drop-before-wrap rule measures `named()`, so this item changes what it measures, the mark included. Tests: `the_short_name_keeps_a_subcommand_after_a_flag` (`--color always nextest run` → `nextest run …`; `--locked check` → `check …`); `the_short_name_marks_dropped_arguments_only_when_it_drops_some` (`cargo check --workspace` → `cargo check …`; `cargo check` → `cargo check`).
4. **One blank cell after the summary title** (`crates/cargo-tile/src/constants.rs:166` `SUMMARY_CELL_TITLE = " summary"`, written verbatim by `GridLines::add_titled`, `crates/tui_pane/src/pane/frame.rs:481`; every shot L1 reads `┌ summary───`). Rule: one blank cell before and after the title, as the sccache label and toast title have. cargo-handler's own summary title is out of this phase (author's call: the approved evidence is cargo-tile's). Test: `the_summary_title_has_a_blank_cell_after_it` — the top row reads `┌ summary ─`.
5. **Short view names the one ancestor that ran the session** (`render.rs` `drawn_ancestry` and `shortened`, which today keep the whole chain in Short view and only drop arguments; `census::command_name`). Evidence, the user's shot `/tmp/claude-1000/-home-natepiano-rust-cargo-liner/4dbdb78e-a83b-4248-b8f9-ff278dcbc60c/images/2.png`: a hana-linux-2 cell whose Short-view ancestry runs `Runner.Listener` (519409), `Runner.Worker spawnclient` (1556662) and `bash` (1558245), each a full `/nix/store/…` path wrapping over two or three rows. The user, 2026-10-09 (via the showrunner): "In compact mode it's not helpful to see the full stack of items that drove the session - it woul be ideal to just show the command that most likely was the important session runner - maybe that's not the last if it's always bash, right?" Rules, set by the showrunner:
   - In Short view the ancestry block is one row: the nearest ancestor that is neither a shell nor a pure exec wrapper. Shells: today's `TRANSPARENT_PROCESS_NAMES` (`crates/cargo-tile/src/constants.rs`, which includes `login`, so a terminal session's login is never the one row; showrunner, 2026-10-09) plus `nu`, kept as that one list rather than a second copy. Wrappers: `env`, `nice`, `nohup`, `setsid`, `stdbuf`, `timeout`, `time`, `sudo`, `doas`, `script`, `ionice`, `chrt`, `taskset`, `caffeinate` (showrunner's ten plus four, accepted), a new list in the same file. Both are matched on the program's file name. When no ancestor qualifies, the block is drawn as today. Adding `nu` to the shells reaches the census (`crates/cargo-tile/src/census/scan.rs` reads `TRANSPARENT_PROCESS_NAMES`); test `nu_is_a_transparent_launcher`.
   - In Short view a program is named by its file name, never its path, in ancestry rows and in table rows alike: `/nix/store/…/Runner.Worker spawnclient 166 175` reads `Runner.Worker …`. The ` …` follows item 3's mark rule.
   - Long view keeps the whole chain with full command lines, unchanged.
   - Short view gives up the foot's pid link to the first row's parent when a shell sits between them; Long view keeps it (showrunner's call).
   Write the failing test from the shot's chain first. Tests: `the_short_ancestry_is_the_nearest_session_runner` (the shot's chain `Runner.Listener` → `Runner.Worker spawnclient 166 175` → `bash` → `cargo` gives one row `1556662 Runner.Worker …`; `claude` → `bash` → `cargo` gives `claude`; `ghostty` → `zsh` → `cargo` gives `ghostty`; `env` and `nohup` between them are skipped too); `a_short_view_program_is_named_by_its_file_name` (an ancestry row and a table row whose program is a `/nix/store/…` path); `the_long_ancestry_keeps_every_level` (the shot's chain in Long view, unchanged).
6. **A cell's own command comes first** (phase 10 design check, toast200 L42: the hana_catalyst-lfo cell, cut to one row, shows a child `909889 909862 … cargo` that started 0 s ago, while its own `909862 cargo nextest run`, the row the summary names and the cell's first row in deep126x80 L45 and toast64 L37, hides behind `…`). Rule: in a command cell, the cell's own command row (the lead) is drawn first and its children follow, so a cell cut to one row shows its own command (author's call). Test: `a_cell_cut_to_one_row_shows_its_own_command`.
7. **One way to cut a directory heading** (phase 10 design check, w40 L3 `[natepiano] …/tool-based-ui-trunk` against L7 `…/rust/tool-based-ui-frame-time`, which is 31 cells and fits a 36-cell line whole; toast40 L3 against L11). Rule: a heading drops its `[account]` tag first and shows the path whole when it fits; only a path that does not fit alone loses leading segments behind `…` (author's call). Test: `a_heading_drops_its_tag_before_cutting_its_path` — at inner width 38: `~/rust/tool-based-ui-frame-time` whole, `…/tool-based-ui-geometry-material` cut.

Changelog: one Changed line under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/cargo-tile/src/render.rs` — progress per command, ancestry form, the Short-view session runner, tests.
- `crates/cargo-tile/src/census/command_text.rs` — `named` and the program's file name, their tests.
- `crates/cargo-tile/src/census/scan.rs` — the `nu` classification test.
- `crates/cargo-tile/src/constants.rs` — `SUMMARY_CELL_TITLE`, the shell and wrapper lists, any new literal.
- `crates/cargo-tile/CHANGELOG.md` — the Changed line.

**Seats:** 1 writer — every item is cargo-tile text drawing, and `render.rs` is the hub of all seven.
- `impl` — `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/census/command_text.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/CHANGELOG.md`; runs the one lint and the four suites.

**Constraints from prior phases:** Phase 6 and 7: `ELISION` is cargo-tile's one mark; `AncestryLevel::AncestorAfterElision` and `mark_ancestry_cut` are the ancestry cut mark. Phase 9: tables are planned before drawing (`ProcessTablePlan`), and a cut command fills its column before its mark. Phase 13 makes column offsets frame-wide through its staged grid, moves compiler and runs after command, insets every row one cell from the right frame, and drops columns before a command name wraps; the `state` slot in item 1 follows Phase 13's optional-column rule. No new public item. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- These earlier table tests pass unchanged: Phase 9's `a_table_that_yields_one_gap_yields_them_all_before_its_foot`, `a_fully_drawn_table_keeps_every_inter_group_gap` and `a_cut_command_fills_its_column_before_the_mark`; Phase 13's `stacked_command_cells_share_one_column_grid` and `columns_drop_before_a_command_name_wraps`.
- Unit director's captures, the eight shots (w200, w64, w48, w40, toast200, toast64, toast40, deep126x80): no two commands share a percent, one ancestry form, `cargo nextest run`'s inner cargo named with its subcommand, a Short-view name ending in ` …` exactly when it dropped arguments, a Short-view ancestry of one row naming the session runner by file name, no `/nix/store` path in Short view, and `┌ summary ─`.
- A fresh helper's design check of those shots: pass.

### Phase 15 — The toast reads in plain words, and a word that fits a line is never split  · status: todo

#### Work Order

**Goal:** The capture toast says what it does in plain words with its undo command highlighted on one line, its check mark draws, it keeps one cleared row above and below, and a word that fits a fresh line moves there whole.

**Spec:**

Evidence: toast shots in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, text beside each in its `.txt`. Reproduce each item in a failing test before changing anything.

Preflight, the unit director's, before dispatch: on the Mac (`ssh mac`), read the font Ghostty draws with (`font-family` in its config, else its default) and check that font and the fallback fonts Ghostty uses for U+2713 with a fontTools cmap read. When the Mac draws it, item 2 is the capture renderer's font fallback: the unit director's tooling at `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots5.py` (`ImageFont.truetype` loads Hack Nerd Font Mono alone) gains a per-glyph fallback to a font that holds U+2713, the re-shot toast200, toast64 and toast40 show the mark, and item 2 leaves this Work Order before dispatch. When the Mac lacks it, item 2 stays as written below, with the glyph the preflight names.

1. **The toast reads in plain words** (`crates/cargo-tile/src/capture.rs:53–56`). Today: "In front of cargo for …; \`cargo tile uninstall\` gives cargo its name back.", with literal backticks, and the command breaks across lines at 200 and 40 (toast40 L91–L96). Rule: the body says what the user gets and how to undo it in plain words, with no shim mechanics and no backticks. The toolchain name stays. `cargo tile uninstall` is drawn in a highlight ink and never breaks across lines. The toast API cannot carry that today: `ToastBody` (`crates/tui_pane/src/toasts/body.rs`) is flattened to text and optional per-line colours before rendering, and `crates/tui_pane/src/wrap.rs` breaks at every space, even inside one styled span. So a body is built of styled runs, each saying whether it may break inside (a named type, for example `RunBreaks::{AtSpaces, Never}`, never a bool), carried through `toasts/commands.rs` and `toasts/view.rs` and measured and drawn through the wrapper by `toasts/render/card.rs`. `ToastView.body_line_colors` becomes a named render-body type, not an `Option`. Today's text bodies keep working unchanged. Tests: `a_nonbreaking_run_moves_whole_to_the_next_line` on the shared API in `tui_pane`; `the_capture_toast_names_its_undo_command_on_one_line` at card widths 36 and 58 in cargo-tile.
2. **The toast's check mark draws** (`crates/tui_pane/src/toasts/render/card.rs:89` writes U+2713; it shows as an empty box in the shots). Rule, when the preflight finds the Mac's terminal font lacks U+2713: draw the glyph the preflight names, one the font has. Test: `the_toast_mark_is_the_glyph_the_terminal_font_has`.
3. **A word moves whole to a fresh line when it fits there** (`crates/tui_pane/src/wrap.rs`, Phase 7's wrapper; showrunner, 2026-10-09). toast64 and toast200 L45–L46 split the toolchain name at a hyphen, `nightly-2026-09-03-x86_64-unknown-` / `linux-gnu.`, though the whole 44-cell word fits on the card's 56-cell line. Rule: a word that does not fit in what is left of a line holding text moves whole to the next line whenever it fits on a line by itself; only a word longer than a whole line is broken, at a hyphen if it has one, else at the width. The wrapper also lays out cargo-tile's table rows (`process_row`), so this changes row heights and command cuts there. Test: `a_hyphenated_word_moves_whole_when_it_fits_a_fresh_line` — the toolchain name after a short sentence at width 56 lands whole on the second line; a word longer than the width still breaks at its last hyphen that fits.
4. **A toast keeps one cleared row above and below** (`crates/tui_pane/src/toasts/render/drawing.rs` ~75 places cards with no stack gap and owns their outer placement; `card.rs` clears `TOAST_SIDE_GAP` cells left and right only, ~146–175; `layout.rs` sizes the card). Phase 9's design check: the card's bottom border (toast64 L48) sits on the row directly above the frame's bottom border (L49), and its top border sits directly under text (toast64 L44 under the heading at L43; toast40 L40 under the column header). Rule: the card is placed one row above the frame's bottom border, and one cleared row is drawn above its top border, matching the side gap; a stack of toasts keeps the same one-row gap between cards. Name the gap in `crates/tui_pane/src/constants.rs` beside `TOAST_SIDE_GAP`. Test, in `drawing.rs`: `a_toast_keeps_one_cleared_row_above_and_below` — a card drawn over filled text: the rows directly above and below it are blank across the card's width plus its side gaps, and the frame border is untouched.

Changelogs: one Changed line each under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md` and `crates/tui_pane/CHANGELOG.md`.

**Files:**
- `crates/tui_pane/src/toasts/body.rs`, `crates/tui_pane/src/toasts/commands.rs`, `crates/tui_pane/src/toasts/view.rs` — styled runs and the render body.
- `crates/tui_pane/src/toasts/toast.rs`, `crates/tui_pane/src/toasts/manager.rs`, `crates/tui_pane/src/toasts/lifecycle.rs` — only where the internal render body must pass through.
- `crates/tui_pane/src/toasts/render/card.rs` — runs measured and drawn; the mark glyph only if the preflight says so.
- `crates/tui_pane/src/toasts/render/drawing.rs`, `crates/tui_pane/src/toasts/render/layout.rs`, `crates/tui_pane/src/constants.rs` — the toast's cleared rows and their test.
- `crates/tui_pane/src/wrap.rs` — nonbreaking runs, whole-word move to a fresh line, their tests.
- `crates/cargo-tile/src/capture.rs` — the toast body and its test.
- `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md` — Changed lines.

**Seats:** 2 writers — the toast API and wrapper in `tui_pane`, and the cargo-tile body that uses them.
- `impl` — items 1's shared API, 2, 3 and 4: every `crates/tui_pane/` path above and the tui_pane changelog. Lands the styled-run type and its constructor first and posts their names on the board.
- `test` — opens as impl; item 1's cargo-tile half: `crates/cargo-tile/src/capture.rs` and the cargo-tile changelog. Writes `the_capture_toast_names_its_undo_command_on_one_line` failing first, then builds the body on `impl`'s posted API.

**Constraints from prior phases:** Phase 7's `#[expect(clippy::expect_used, reason = "tests should panic on unexpected values")]` in `crates/tui_pane/src/wrap.rs` is user-approved and stays. Phase 8: toasts keep one cleared cell each side (`TOAST_SIDE_GAP`), and `toast_card_width` decides card width. Phase 9's `a_cut_command_fills_its_column_before_the_mark` and table gap tests pin row heights the wrapper now changes. Phase 13's shared grid and drop-before-wrap rule measure wrapped rows. New public items are only the styled-run types and constructors; no existing `tui_pane` public signature is removed, and the public `ToastBody` keeps its variants and methods: styled runs enter through a new constructor and an internal render body, so no caller's exhaustive match breaks. `ToastView`'s existing `Option` linger and countdown states are outside this phase's items and stay. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- Unit director's frame probe over toast64 and w200 in motion (this is the last phase that changes wrapping): 0 moving pieces with a blank row above their foot while their content hides rows.
- These earlier table tests pass unchanged: Phase 9's `a_table_that_yields_one_gap_yields_them_all_before_its_foot`, `a_fully_drawn_table_keeps_every_inter_group_gap` and `a_cut_command_fills_its_column_before_the_mark`; Phase 13's `stacked_command_cells_share_one_column_grid` and `columns_drop_before_a_command_name_wraps`.
- Unit director's captures, the eight shots: the toast in plain words with its command and toolchain name each on one line, its check mark drawn, and a cleared row above and below it.
- A fresh helper's design check of those shots: pass.

### Phase 16 — Readable inks, and a foot that fades  · status: todo

#### Work Order

**Goal:** Every ink on the grid meets its contrast floor on both grounds of every built-in theme, a finished row stays readable through the first half of its fade, and a fading cell's foot, and the summary's, fades with its contents.

**Spec:**

Evidence: shots in `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, text beside each in its `.txt`. Contrast is measured from cell colours, never PNG pixels (Phase 7 gotcha). Reproduce each item in a failing test before changing anything.

Inks (built-in themes Default Dark and Default Light, transparent off; grounds are the ones `tui_pane` paints, `pane_background(true)` and `pane_background(false)`, `crates/tui_pane/src/pane/chrome.rs`; shots read #0e0e11 and #2e2e39 dark, #e0e0e9 light summary):
1. **Pid identity colours** (`crates/cargo-tile/src/theme/mod.rs` `FAMILY_COLORS` 25, `family_color` 105: one solarized set for both appearances; light 2.44 to 3.47:1, dark magenta 2.95:1 on the summary grounds). Rule: every colour `family_color` can hand out measures 4.5:1 or more on both grounds of the active theme, and the clearance and more-than-one-family tests still pass. One set cannot meet it in both appearances (it would need relative luminance ≥ 0.30 on #2e2e39 and ≤ 0.13 on #e0e0e9), so the set follows the appearance. Test: `every_family_colour_meets_contrast_on_both_grounds_in_every_built_in_theme`.
2. **The red count in the foot readout** (`crates/tui_pane/src/tiles/draw.rs` `rows_readout_lines` 511 writes the count in `error_color()`; Default Dark `error = Red` (`crates/cargo-tile/src/theme/builtins.rs:86`) draws #cd0000: 3.30:1 on #0e0e11, 2.30:1 on #2e2e39). Rule: the over-budget count, and the `@ W` width written in the same ink, measure 4.5:1 or more on both grounds in every built-in theme of cargo-tile, and of cargo-handler, which draws the same readout (`crates/cargo-handler/src/theme.rs:370`). Test: `the_readout_count_meets_contrast_on_both_grounds_in_every_built_in_theme`, one in each app.
3. **Light summary green** (Default Light `success = Rgb(0, 120, 0)`, `builtins.rs:136`: 4.33:1 on #e0e0e9; the shot read 4.05:1 from pixels). Rule: every ink a table row draws at fade 0 (`text_default`, `label_color`, `success_color`, `warning_color`, `secondary_text_color`) measures 4.5:1 or more on both grounds in every built-in theme. Test: `table_inks_meet_contrast_on_both_grounds_in_every_built_in_theme`, next to `status_line_text_meets_contrast_in_every_built_in_theme` (builtins.rs 436, whose `contrast_ratio` it reuses).

Fade:
4. **Dimmed finished rows** (`crates/cargo-tile/src/roster.rs` `faded_at` 137 carries a finished row toward its ground evenly over `DEFAULT_FADE_SECONDS`; the summary's #607374 on #2e2e39 is that blend about half way, 2.95:1). Rule (author's call; the fade stays an exit): a finished row's ink stays at 3:1 or more on its ground until half of the fade has run, then reaches the ground at the end; summary and cells alike. Measured against the inks items 1 to 3 land. Test: `a_finished_row_stays_readable_through_the_first_half_of_its_fade`.
5. **A fading cell's foot fades with it** (`draw.rs` `draw_cell` 387 → `draw_rows_readout` 474 writes full-strength inks; deep L59–L69 show a near-black cell with a bright `content rows: 10  r/c: 9/124`). Rule: every span of the foot readout, and of the summary foot, is drawn as `blend_color(ink, ground, fade)` with the cell's own fade and ground, the same fade its contents use. `TileCells` gains one provided method returning the cell's fade as a named state, `CellReadoutFade::{Unfaded, Fading(CellFadeProgress)}` with `CellFadeProgress` a named amount, never an `Option<u8>`; its default is `Unfaded`, so cargo-handler (`crates/cargo-handler/src/render.rs`) and `draw_tile_cell` keep today's drawing with no edit. cargo-tile answers a command cell from the one function `draw_group` uses (`heading_fade(&rows).min(group.lead.faded())`, render.rs 791) and the summary from `heading_fade` over `summary_rows(...)`, the rows the summary draws. Existing `Option` states in `TileCells` this item does not touch (`group_title`) stay as they are. Tests: `draw.rs` `a_fading_cells_readout_fades_with_it` through `draw_tile_grid` with `StubCells`; `render.rs` `a_finished_commands_cell_reports_the_fade_its_contents_use` and `the_summary_foot_fades_with_its_rows`.

Changelogs: one Changed line each under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md` and `crates/tui_pane/CHANGELOG.md`, and in `crates/cargo-handler/CHANGELOG.md` when its theme changes.

**Files:**
- `crates/cargo-tile/src/theme/mod.rs`, `crates/cargo-tile/src/theme/builtins.rs`, `crates/cargo-tile/themes/*.toml` — family colours as theme roles, inks, contrast tests.
- `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/*.toml` — readout red, its test.
- `crates/cargo-tile/src/roster.rs` — fade curve and its test.
- `crates/cargo-tile/src/render.rs` — the cell and summary fade answers, their tests.
- `crates/tui_pane/src/tiles/draw.rs` — `CellReadoutFade`, the provided method, the faded foot, render test.
- `crates/cargo-tile/src/constants.rs` — any new literal.
- `crates/cargo-tile/CHANGELOG.md`, `crates/tui_pane/CHANGELOG.md`, `crates/cargo-handler/CHANGELOG.md` — Changed lines.

**Seats:** 2 writers — theme data and contrast tests, and the fade path from roster to foot.
- `impl` — items 1 to 3: `crates/cargo-tile/src/theme/`, `crates/cargo-tile/themes/`, `crates/cargo-handler/src/theme.rs`, `crates/cargo-handler/themes/`, the cargo-handler changelog. Posts on the board when the inks are final.
- `test` — opens as impl; items 4 and 5: `crates/cargo-tile/src/roster.rs`, `crates/cargo-tile/src/render.rs`, `crates/tui_pane/src/tiles/draw.rs`, the cargo-tile and tui_pane changelogs; hub: `crates/cargo-tile/src/constants.rs`. Does item 5 first; starts item 4 only after `impl` posts its inks final, because item 4 measures them.

**Constraints from prior phases:** Theme installation does not keep the resolved `Appearance`, so `family_color` cannot ask it: encode the family hues as cargo-tile theme roles in `builtins.rs` and each theme file, so each theme carries its own set and the active theme selects it. Phase 4: no ink changed then; any ink change now is checked on both themes, transparent on and off. `theme/state.rs` keeps its four `#[expect(clippy::expect_used)]`. Phase 7: `status_line_text_meets_contrast_in_every_built_in_theme` holds status inks at `MIN_STATUS_LINE_TEXT_CONTRAST` = 4.5, and named colours resolve through its xterm table. Phase 13 makes column offsets frame-wide and insets every row one cell from the right frame. New public items are only `CellReadoutFade`, `CellFadeProgress` and the provided `TileCells` method; no existing `tui_pane` public signature changes. cargo-handler's cell sizes do not change. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test takes a second or more.
- Unit director's captures, the eight shots, on both themes, transparent off and on: every measured ink at its floor from cell colours, and a fading cell's foot as dim as its contents.
- A fresh helper's design check of those shots: pass.

### Phase 17 — Slow tests start first  · status: todo

#### Work Order

**Goal:** On each machine, every non-reader test observed at 0.8 s or more in any measurement run starts before every faster non-reader test in a full `cargo nextest run`; the seven reader scenarios keep their existing priority and one-at-a-time rules. Ordering only: no test is rewritten (user's words: "for now").

**Spec:**

1. **Measure before.** On each machine run the suite CI runs, `cargo nextest run --all-features --workspace --exclude cargo-mend --tests`, three times with JUnit turned on from outside the repo config: a file `junit.toml` holding `[profile.default.junit]` / `path = "junit.xml"`, passed as `--tool-config-file tile-fixes:<absolute path>/junit.toml`. Each `testcase` then carries its start `timestamp` and `time`. Every run writes the same `target/nextest/default/junit.xml`, so before the next run copy it to `<machine>-<before|after>-<n>.xml` (machine `natedev` or `mac`, `n` 1 to 3, or 4 to 6 for a repeated after batch) in the delegate session directory; the Mac lane copies each file there over `scp` as it finishes, so every run's evidence survives. Record the wall time of each run and, per test, its slowest time. On the Mac work in a clone outside `/tmp`, reached with `ssh mac`: `git clone git@github.com:natepiano/cargo-liner.git /Users/natemccoy/rust/cargo-liner-measure` if it is absent, then before every batch of runs `git -C /Users/natemccoy/rust/cargo-liner-measure fetch origin remove-running-section-tile-fixes`, `git -C … checkout --detach FETCH_HEAD`, and confirm `git rev-parse HEAD` equals the phase tip on natedev and `git status --short` is empty. (`/Users/natemccoy/rust/cargo-liner` is the user's checkout on `main`; leave it alone.)
2. **The slow set** is every test at 0.8 s or more in any of the six runs, less the seven reader scenarios the config already orders. Known members to confirm by name with `cargo nextest list`: in `cargo-tile::unit_tests`, `progress::capture::…::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`, eight in `shim_registration::wire::`, two in `hook::`; two each in `cargo-tile::shim_modes` and `cargo-tile::cli_lifecycle`; in `cargo-handler`, `census::codex::…::reads_threads_by_id_whoever_started_them` and `…::reads_interactive_threads_created_in_the_span`; in `tui_pane`, `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw`.
3. **One override.** Append to `.config/nextest.toml` one `[[profile.default.overrides]]` with `priority = 80` and a `filter` that names each slow test exactly: `binary_id(<id>) & (test(=<name>) | …)` per binary, joined with `|`. No regular expressions, so a test added later is not swept in by accident. A comment above it says what the list is, the threshold, the date measured, and how to re-measure. nextest resolves each setting from the first override that sets it, so the two existing priorities (100, 90), the `cargo-tile-readers` group and both `slow-timeout` entries keep working unchanged; the new override sets `priority` alone and names none of the reader scenarios.
4. **Measure after**, the same three runs per machine. A non-reader test that crosses 0.8 s in an after run and is not in the filter joins it, and the three after runs repeat once on both machines; a second newcomer is reported in the summary rather than chased.

**Files:**
- `.config/nextest.toml` — one override and its comment. In scope for this unit by the showrunner's word (2026-10-08), though the Units row does not list it.

**Seats:** 1 writer + 1 tester — one configuration owner and two measurement lanes on a byte-identical candidate.
- `impl` — owns `.config/nextest.toml`, publishes its `sha256sum` on the board after each edit, and runs every natedev before and after run; checks every exact filter name.
- `test` — no source file; copies the candidate config to the Mac clone (`scp` over `ssh mac` into `/Users/natemccoy/rust/cargo-liner-measure/.config/nextest.toml` after the checkout), verifies the same sha256 before each after-run batch, runs every Mac run, and hands `impl` the named JUnit files, the slow set and the start order. The before runs use the clean phase tip.

**Constraints from prior phases:** This phase runs last, after every phase that adds tests or changes shared grid code, so its timings measure the suite the run ships. Phases 1 to 16 added render tests to `tui_pane` and `cargo-tile`; each gate asked for under a second for its own tests, and this phase's line is 0.8 s, so the six measurements decide whether any of them joins the slow set. Seen over a second during Phase 8's runs under load: `tui_pane` `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw` (1.52 s) and several reader and `shim_modes` scenarios.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, after the config edit: green.
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, the source-switch reader is first, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.
