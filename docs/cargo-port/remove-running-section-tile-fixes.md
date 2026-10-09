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

#### Work Order

**Goal:** No settled command cell, and no resident piece in flight, is drawn too short to show a process: cargo-tile's cells are at least six rows high, a command the window has no such cell for is counted in the summary until there is room, a table cut at its foot says so, and a toast keeps an even one-cell gap on each side. Entering and leaving pieces stay transition fragments (item 1).

**Spec:**

Evidence (Phase 5's design check, 50-row shots at 40, 48 and 64 columns, cargo-tile's `initial_rows = 12`): a live command cell is drawn two or three rows high, a column heading and a foot with no process row, while the summary above it keeps blank rows. Reproduce each cause below in a failing test before changing anything.

1. **The floor is true at the boundary** (`tui_pane`, `grid.rs`). `fits` (1899) accepts a column of `shared_run(2, 3) == 5` rows, but `shares` (1824) can return `[3, 2]` unchanged (1882 to 1884) and `share_borders` (`crates/tui_pane/src/pane/frame.rs:156` to 166) gives the shared border's row only to the piece that is not at the bottom, so the bottom piece is two rows high, under `TileSettings::min_tile_height` (`tiles/settings.rs:18`, default 3 at 42). Rule: every piece a settled arrangement draws, and every resident piece (neither entering nor leaving) in each snapshot of a step through `drawing_at`, is at least `min_tile_height` rows high with its frame; make `fits` and `shares` agree on what a column of touching pieces needs, at the exact boundary and with a widened summary. An entering or leaving piece is a transition fragment: it still collapses toward a one-row divider, and Phase 3's `a_departing_cell_keeps_its_title_while_its_border_is_visible` passes unchanged.
2. **cargo-tile sets its own floor.** No path reaches `min_tile_height` from an app: the one setter is `TileGrid::set_min_tile_width` (`grid.rs:497`), called at `crates/cargo-tile/src/app.rs:200`. Add `pub fn set_min_tile_height(&mut self, height: u16)` beside it (it writes `settings.min_tile_height`, never below the framework minimum) and call it beside line 200 with a new `MIN_CELL_HEIGHT = 6` in `crates/cargo-tile/src/constants.rs`: two frame rows, the table header (`column_header`, drawn by `draw_process_table`), a directory heading (`draw_path_group`), one process row (`process_row`), and the foot readout's row (`draw_rows_readout` in `crates/tui_pane/src/tiles/draw.rs`). `cargo-handler` keeps the framework minimum.
3. **A command without a cell.** A cell `fits` refuses for height is not opened, as with the width floor: its command is counted in the summary and opens a cell by the ordinary `sync` rule once there is room. Reuse Phase 5's `holds_in`, the summary alone and that refusal path; add no second refusal state. `sync` and `admitted` already refuse through `fits` (`a_command_refused_in_a_small_window_opens_when_there_is_room` proves the width form), so this item adds only its height test and no production path.
4. **A cut table is marked** (cargo-tile `render.rs`; phase 6's and phase 7's design checks). Generalize `mark_ancestry_cut` (`render.rs:977`) into one helper named for what it does (a line that ends where content was cut) and use it for ancestry and for both tables, `TableKind::Command` and `TableKind::Summary`. When a table's last visible row loses wrapped command lines at the cell's foot, the last line drawn ends in the mark (`cargo…`). When whole later rows are hidden (the summary listing 4 to 6 of 8 running builds), the last line drawn ends in the mark too, so a hidden row is never silent. Tests: `a_cut_command_row_ends_in_the_mark` (a command cell one row short of its table draws the mark on its last command line); `a_summary_with_hidden_rows_ends_in_the_mark` (eight builds in a summary with room for five); `a_wrapped_summary_command_cut_at_the_foot_ends_in_the_mark` (40 columns, `cargo` / `clippy` with the second line cut).
5. **An empty column yields** (cargo-tile `render.rs`; phase 6's design check). A command cell keeps `compiler` and `runs` today through `TableKind::shows_invocation_detail`. A column is empty when every row of the cell holds `CompilerObservation::None` (for `compiler`) or `Measurement::Reading(0)` (for `runs`); `Unknown` and `Unavailable` draw `--` and keep their column. An empty column is not drawn, and `command` takes its cells before any command wraps. Test `an_empty_column_yields_its_cells_to_the_command`: at 62 inner cells with no compiler value, `cargo check` stays on one line. `a_commands_own_cell_keeps_them` and the two column-count tests keep their rule for cells with a value and change only where their fixture is empty; name each changed expectation in the summary.
6. **A toast keeps an even gap** (showrunner's call from the UX guide, 2026-10-09). A toast keeps one cleared cell on each side, left and right, inside the area `render_toasts` is given (`frame_inner(body)`), so it neither breaks the cell divider it covers on its left nor sits against the frame line on its right. Reserve those two cells before the card width is computed (`toast_card_width` in `toasts/manager.rs`, placement `x = area.x + area.width - layout.width` in `toasts/render/layout.rs:52` and `:98`, the clear rectangle in `toasts/render/card.rs:39-44`), so wrapping, height and entrance timing count the width the card is actually drawn at; the full-width slack rule (`TOAST_FULL_WIDTH_SLACK`) keeps its meaning with the gap included. `ToastDrawAreaWidth` keeps storing the area width it is named for. Test in `toasts/render/drawing.rs`, reading the drawn buffer: `a_toast_keeps_one_cleared_cell_on_each_side` (a narrow and a full-width card in an offset area: the cell left of the card and the cell right of it are cleared, both inside the area). Phase 7's width-state tests (`mutable_settings_keep_toast_creation_in_sync_with_the_drawn_area`, `a_full_width_toast_does_not_clear_left_of_its_offset_area`, the slack 7 and 8 tests) change only where the two-cell gap moves their expected numbers; name each in the summary.

Tests, all pure:
- `grid.rs`: `a_column_at_the_exact_floor_draws_no_piece_under_it` (two and three touching pieces at `shared_run` exactly, and one row over); `a_widened_summary_leaves_no_piece_under_the_floor`; `no_resident_piece_is_under_the_floor_in_flight` (the resident pieces in every snapshot of an opening and a closing step); `fits_holds_at_the_height_floor_and_fails_one_row_under` with the floor set to 6.
- `grid.rs` or `draw.rs`: `a_command_refused_for_height_opens_when_there_is_room` (synced in a short area with more commands than cells, then in a tall one).
- `crates/cargo-tile/src/render.rs`, through the real `Cells`: `every_command_cell_shows_a_process` — at 40, 48 and 64 columns by 50 rows, with the summary asking twelve content rows and six commands: each command cell drawn has its column heading, a directory heading, a process row and its foot; the same at 200 by 50 with ten commands and at 126 by 80 with a deep ancestry chain, settled.

README: the `[tiles]` section of `crates/cargo-tile/README.md` gains one sentence: a command cell is never shorter than six rows. Changelogs: one Changed line each under `## [Unreleased]`.

**Carried from phase 7's design check** (shots `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph7/shots/w200.png`, `w126.png`, `w48.png`, `deep126x80.png`): a cell given three rows draws only its column header, heading and readout; the bottom cell given two rows loses its heading; at 200 columns three 10-cell columns show empty boxes and pid-only fragments; over-budget cells in deep126x80 leave a blank row above the readout. Each settled one is a cell that should show a process or not be drawn; a fragment of a cell entering or leaving its column is a transition fragment under item 1 and stays. Also carried (shots `w48.png`, `w40.png`, `toast40.png`, `toast64.png`, `w126.png`): the summary lists 4 to 6 of 8 running builds with nothing saying the rest are hidden, and at 40 columns a summary entry's command wraps to `cargo` / `clippy` and the row budget drops the second line unmarked (item 4). Toast spacing shots: `toast200.png`, `toast64.png`, `toast40.png` (item 6).

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — `shares`, `fits`, `set_min_tile_height`, tests.
- `crates/tui_pane/src/pane/frame.rs` — `share_borders`, only if the rule belongs there.
- `crates/tui_pane/src/tiles/draw.rs` — a render test, if the refusal test lives there.
- `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/constants.rs` — `MIN_CELL_HEIGHT` and its one call.
- `crates/cargo-tile/src/render.rs` — items 4 and 5, the render tests.
- `crates/tui_pane/src/toasts/manager.rs`, `crates/tui_pane/src/toasts/render/layout.rs`, `crates/tui_pane/src/toasts/render/card.rs`, `crates/tui_pane/src/toasts/render/drawing.rs`, `crates/tui_pane/src/constants.rs` — item 6 and its test.
- `crates/cargo-tile/README.md`, `crates/tui_pane/CHANGELOG.md`, `crates/cargo-tile/CHANGELOG.md` — the floor.

**Seats:** 2 writers — grid geometry owns the floors; rendering owns every cargo-tile table and the toast.
- `impl` — `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/pane/frame.rs`, `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/src/constants.rs`, the README and both changelogs. Its first edit adds `TileGrid::set_min_tile_height` with its final signature and posts it on the board. It runs the one lint and the four suites after both seats have posted `done`.
- `test` — opens as impl: `crates/tui_pane/src/tiles/draw.rs`, `crates/cargo-tile/src/render.rs`, `crates/tui_pane/src/toasts/manager.rs`, `crates/tui_pane/src/toasts/render/layout.rs`, `crates/tui_pane/src/toasts/render/card.rs`, `crates/tui_pane/src/toasts/render/drawing.rs`, `crates/tui_pane/src/constants.rs`: the refusal test, items 4 to 6 and the render tests. It sends `impl` its changelog lines.

**Constraints from prior phases:** Phase 5 added `TileGrid::set_min_tile_width` and the crate-private `holds_in`, made the grid answer the summary alone itself (`drawing_at` returns one summary piece when `holds_in` fails), and left a command refused a cell counted in the summary until `sync` opens it; this phase reuses all three and their tests pass unchanged. Phase 2 made `TileGrid::drawing_at(area, growth, raw)` the pure entry to a motion; Phase 3 made a closing column close as one piece (`no_piece_in_a_closing_column_changes_height`) and gave each piece a name role; neither changes. Phase 6 changed text inside cells and the toast, nothing in the grid; Phase 7 moved the wrapper to `tui_pane::wrapped` and changed how a command's width is measured (`command_line_width`); it added `AncestryLevel::AncestorAfterElision` (one ancestry row draws `… <nearest>`, two draw the elision row and the nearest), whose short- and deep-ancestry tests pass unchanged while item 4 reuses the mark for tables. Phase 7 also draws toasts inside the tile frame (`tui_pane::render_toasts(frame, &mut app.framework, tui_pane::frame_inner(body))` in both applications), stores the drawn area width as `ToastDrawAreaWidth { Unconstrained, Drawn(u16) }` and derives `Toasts::card_width()` through `toast_card_width(settings, area_width)`; item 6 changes only the width that function returns and where the card is placed. `cargo-handler` draws on the same grid at the framework minimum, so rule 1 fixes its two-row pieces too; a test of its that fails only because of rule 1 is updated for that change only and named in the summary. `set_min_tile_height` is this phase's only new public item. No new `#[allow]` or `#[expect]`: the summary lists each one this phase adds or moves, with its file and line.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, after both seats' last edits, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures beside real cargo commands, 50 rows, at 64, 48 and 40 columns with `initial_rows = 12`, and at 200 columns, plus 126 by 80 with a deep ancestry chain: every command cell drawn shows a process row, a command without a cell is counted in the summary, and a cut table ends in the mark. The first-run toast at 200, 64 and 40 columns, 50 rows: one cleared cell on each side of the card.
- A fresh helper's design check of those eight shots: pass.

### Phase 9 — A cut command fills its column, and a cut cell fills its rows  · status: todo

#### Work Order

**Goal:** A command cut at a cell's foot shows as much of itself as its column holds before the mark, and a cell that hides rows behind the mark leaves no blank row above its foot.

**Spec:**

Evidence (Phase 8's design check after its second repair round; moved here by the hard landing rule; shots `/tmp/claude-1000/-home-natepiano-rust-cargo-liner-tile-fixes/da82452f-3640-4dec-b6c5-0c783d77e79f/scratchpad/ph8/shots/`, text beside each in its `.txt`). Reproduce each case in a failing buffer test before changing anything.

1. **A cut command fills its column** (cargo-tile `render.rs`). The wrapper breaks at a word, and Phase 8's mark follows the last whole word, so a cut command leaves empty cells before its `…`: w64 lines 37 and 42 draw `cargo…` with 4 free cells; w48 line 81 with 5, lines 86 and 91 with 2; w40 line 119 `nextest…` with 2, lines 135 and 140 with 3 and 6; toast40 lines 32 and 37 with 3 and 6. Rule: when a command's wrapped lines are cut at the foot, the last drawn line holds the value's next characters up to one cell short of the command column's width, then the mark (`cargo next…`, `nextest r…`), so the mark sits at the column's far edge. A whole value followed by omitted rows keeps Phase 8's far-edge mark with its free cells, and an exactly fitting whole value keeps Phase 8's wrap onto a free row. Test: `a_cut_command_fills_its_column_before_the_mark` (an 11-cell command column, `cargo nextest run`, one row for it: `cargo next…`).
2. **A cell hiding rows leaves no blank row above its foot** (cargo-tile `render.rs`). deep126x80 lines 29 to 31 and 36 to 38: the `hana-linux-1` and `hana-linux-2` cells, six rows given and fourteen wanted, draw one process row with the mark and then two blank rows above the foot. Find what the hidden content is and why it does not fit (the next directory group needs its gap, its heading and its first process row), and fill the free rows with the next hidden content that fits whole, moving the mark to the last line drawn. A heading is still admitted only with its first process row (Phase 8). Name the cause in the summary. Test: `a_cell_hiding_rows_leaves_no_blank_row_above_its_foot`, the deep126x80 cell at 124 inner cells and six rows.

Changelog: one Changed line under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/cargo-tile/src/render.rs` — both items and their tests.
- `crates/cargo-tile/CHANGELOG.md` — the Changed line.

**Seats:** 1 writer — both items live in cargo-tile's table drawing in one file.
- `impl` — `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/CHANGELOG.md`; runs the one lint and the four suites.

**Constraints from prior phases:** Phase 6 made headings and ancestry whole or marked with the one glyph `ELISION`. Phase 8 made every table cut end in the mark (`mark_cut_line`, `mark_last_drawn_table_line`, `TableContinuation`: a value cut inside itself carries an attached mark, a whole value followed by omitted rows carries it at the command column's far edge, and one that exactly fills its column wraps its last word onto a free row first), admits a directory heading only with its first process row, drops an empty `compiler` or `runs` column, and set cargo-tile's cell floor to `MIN_CELL_HEIGHT` (6). Phase 8's mark tests pass unchanged except where item 1 moves the mark of a cut value, each named in the summary with its old and new text. No new public item. No new `#[allow]` or `#[expect]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures beside real cargo commands, Phase 8's set (64, 48, 40 and 200 columns by 50 rows, 126 by 80 deep, the toast at 200, 64 and 40): every cut command's mark at its column's far edge, and no blank row above a foot while the mark says rows are hidden.
- A fresh helper's design check of those shots: pass.

### Phase 10 — The summary takes the rows it asks for, not a second position  · status: todo

#### Work Order

**Goal:** The summary takes a further position in its column only when it would use more than half of it, so no rows stand blank in the summary while command cells are short of theirs.

**Spec:**

Evidence (Phase 2's design check, 200x50, the tenth command cell opening while the summary asks 12 content rows): the left column settles at 20, 13 and 12 rows where it was 16, 15 and 14; the summary shows seven blank rows; every other column holds four cells of 12 rows.

Cause as read; reproduce it in a failing test before changing anything. `summary_depth` (`grid.rs:1932`) deepens the summary as soon as `summary_share` (1912) at the current depth is under what it asks. `summary_share` calls `shares` (1824), which gives every piece its base share by weight (`apportion`, the summary's weight is its depth) and then serves the summary only from `spare`, the rows its column-mates do not ask for. In a 47-row column of four positions the base share is 11 or 12 rows and the mates want theirs, so a summary asking 13 is one or two rows short. It then takes a whole second position, 23 rows, and gives back only what its two remaining mates ask: 20, 13, 12, with seven rows nobody in that column wants, while the cell it pushed out crowds another column.

Rule:
1. Let `slot` be the summary column's base share for one position at the current depth (`apportion` of equal weights over the column's height). When the summary is short by `slot / 2` rows or fewer, it stays at this depth and takes the shortfall from its mates, the tallest first, one row at a time, never taking a mate under `min_tile_height`. The floor is judged on each mate's final framed rectangle, after `share_borders`, not on its raw share: a piece that is not at the bottom may hold `min_tile_height - 1` raw rows because the shared border supplies its last one (Phase 8's shared-run rule). Name the half as a constant in `crates/tui_pane/src/tiles/constants.rs`.
2. When it is short by more than that, or its mates cannot give the rows without going under the minimum, `summary_depth` deepens as today.
3. `summary_depth` stays a non-decreasing function of what the summary asks, with everything else fixed, so a summary growing a row at a time never flips between depths and back.
4. `queue_with_depth` and the motion code do not change: a depth change is still queued as Phase 2 left it.

Tests, all pure, on settled grids:
- `grid.rs`: `a_summary_one_row_short_takes_it_from_its_mates` — the evidence case: eleven positions, the summary at depth 1 with exactly the rows it asks, three mates sharing the rest, none under the minimum.
- `grid.rs`: `a_summary_far_short_still_takes_a_second_position` — a summary asking a slot and three quarters.
- `grid.rs`: `summary_depth_never_falls_as_the_summary_asks_for_more` — every demand from 1 row to the column's height, at 6, 9, 10 and 13 command cells.
- `grid.rs`: `no_mate_goes_under_the_minimum_to_feed_the_summary` — a bottom mate and a mate that is not at the bottom, each measured framed.
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

**Constraints from prior phases:** Phase 2 made `TileGrid::drawing_at(area, growth, raw)` the pure entry to a motion and made `queue_with_depth` keep every surviving cell in a column it starts or ends the queue in; neither changes here. Phase 3 made a column the step removes close as one piece (`BandPieceMotion::ClosingWithColumn`; test `no_piece_in_a_closing_column_changes_height`, with and without a widened summary over the closing column) and gave each piece a name role (`TilePiece`, `PieceName`, `piece_name`); a change to the summary's rows keeps both, and the `a_title_is_on_screen_once_*` and `an_empty_cell_number_is_on_screen_once_*` render tests pass unchanged. Phase 5 added `TileGrid::set_min_tile_width` and the crate-private `holds_in`, which checks every arrangement still to be drawn, each at its own depth: a summary that stays shallower lets a grid hold in a smaller window, it must never let a wider arrangement still in flight be drawn under the floor, and `holds_in`'s tests pass unchanged. `cargo-handler` draws on the same grid, so its summary follows the same rule. Phase 5 also made the grid answer the summary alone itself (`drawing_at` returns one summary piece when `holds_in` fails), so a grid that does not hold never reaches `shares`. Phase 8 made the height floor true: every piece `shares` returns is at least `min_tile_height` rows once framed, at exact `shared_run` boundaries too, and cargo-tile's floor is `MIN_CELL_HEIGHT` (6) through `TileGrid::set_min_tile_height`; rule 1's "never under `min_tile_height`" rests on that, and Phase 8's floor tests pass unchanged. The `grid.rs` line references above were read before Phase 8: find each function by name. Phase 8 also made a table cut at its foot end in the mark, the summary included (hidden later rows too); a summary given fewer or more rows here keeps that mark, and Phase 8's mark tests pass unchanged. No new public item. No new `#[allow]` or `#[expect]`: the summary lists each one this phase adds or moves, with its file and line.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's capture at 200x50 with seven `+` and seven `-` (it reaches ten cells and four columns), transparent off: in every settled frame the summary shows no blank body row while a cell in its column is short of what it asks, and Phase 2's motion measures still read zero.
- A fresh helper's design check of the settled shots and one motion shot per step, against Phase 2's shots of the same steps: pass.

### Phase 11 — Slow tests start first  · status: todo

#### Work Order

**Goal:** On each machine, every non-reader test observed at 0.8 s or more in any measurement run starts before every faster non-reader test in a full `cargo nextest run`; the seven reader scenarios keep their existing priority and one-at-a-time rules. Ordering only: no test is rewritten (user's words: "for now").

**Spec:**

1. **Measure before.** On each machine run the suite CI runs, `cargo nextest run --all-features --workspace --exclude cargo-mend --tests`, three times with JUnit turned on from outside the repo config: a file `junit.toml` holding `[profile.default.junit]` / `path = "junit.xml"`, passed as `--tool-config-file tile-fixes:<path>/junit.toml`. Each `testcase` then carries its start `timestamp` and `time`. Record the wall time of each run and, per test, its slowest time. On the Mac work in a clone outside `/tmp`, reached with `ssh mac`: `git clone git@github.com:natepiano/cargo-liner.git /Users/natemccoy/rust/cargo-liner-measure` if it is absent, then before every batch of runs `git -C /Users/natemccoy/rust/cargo-liner-measure fetch origin remove-running-section-tile-fixes`, `git -C … checkout --detach FETCH_HEAD`, and confirm `git rev-parse HEAD` equals the phase tip on natedev and `git status --short` is empty. (`/Users/natemccoy/rust/cargo-liner` is the user's checkout on `main`; leave it alone.)
2. **The slow set** is every test at 0.8 s or more in any of the six runs, less the seven reader scenarios the config already orders. Known members to confirm by name with `cargo nextest list`: in `cargo-tile::unit_tests`, `progress::capture::…::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`, eight in `shim_registration::wire::`, two in `hook::`; two each in `cargo-tile::shim_modes` and `cargo-tile::cli_lifecycle`; in `cargo-handler`, `census::codex::…::reads_threads_by_id_whoever_started_them` and `…::reads_interactive_threads_created_in_the_span`; in `tui_pane`, `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw`.
3. **One override.** Append to `.config/nextest.toml` one `[[profile.default.overrides]]` with `priority = 80` and a `filter` that names each slow test exactly: `binary_id(<id>) & (test(=<name>) | …)` per binary, joined with `|`. No regular expressions, so a test added later is not swept in by accident. A comment above it says what the list is, the threshold, the date measured, and how to re-measure. nextest resolves each setting from the first override that sets it, so the two existing priorities (100, 90), the `cargo-tile-readers` group and both `slow-timeout` entries keep working unchanged; the new override sets `priority` alone and names none of the reader scenarios.
4. **Measure after**, the same three runs per machine. A non-reader test that crosses 0.8 s in an after run and is not in the filter joins it, and the three after runs repeat once on both machines; a second newcomer is reported in the summary rather than chased.

**Files:**
- `.config/nextest.toml` — one override and its comment. In scope for this unit by the showrunner's word (2026-10-08), though the Units row does not list it.

**Seats:** 1 writer + 1 tester — one configuration owner and two measurement lanes that run at once.
- `impl` — `.config/nextest.toml` and every natedev before and after run; checks every exact filter name.
- `test` — no source file; every Mac before and after run in the clone above at the phase tip, handing `impl` the JUnit files, the slow set and the start order.

**Constraints from prior phases:** This phase runs last, after every phase that adds tests or changes shared grid code, so its timings measure the suite the run ships. Phases 1 to 6 added render tests to `tui_pane` and `cargo-tile`; each gate asked for under a second, and this phase's line is 0.8 s, so the six measurements decide whether any of them joins the slow set.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, after the config edit: green.
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, the source-switch reader is first, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.
