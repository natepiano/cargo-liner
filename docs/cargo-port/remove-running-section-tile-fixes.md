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
  - `crates/tui_pane/src/app_config/theme_install.rs` — `install_theme` (26), `resolve_appearance` (42); `crates/tui_pane/src/theme/` — `state.rs` (`ThemeState`, `set_active_theme` 141), `resolver.rs` (`AppearanceMode` 23, `resolve_active` 105), `poller.rs` (`spawn_appearance_poller` 180).
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

### Phase 4 — `auto` follows the system's light or dark setting  · status: todo

#### Work Order

**Goal:** With `[appearance] mode = "auto"`, cargo-tile and cargo-handler draw the light theme on a light system and the dark theme on a dark one, from the first frame and when the system changes while they run, so every value reads on a light terminal with transparent on and off.

**Spec:**

The user's report (2026-10-08, cargo-tile on a Mac in light mode): the summary's memory total could not be read, "it's because it's white".

Cause as read; reproduce it in a test first. The default mode is `"auto"` (`DEFAULT_APPEARANCE_MODE`, `crates/tui_pane/src/app_config/constants.rs:19`), and cargo-tile's README says `auto follows the terminal` (line 77). But `install_theme` (`app_config/theme_install.rs:26`) and `apply_settings` (`app_settings/step.rs:143`) both call `resolve_appearance` (`theme_install.rs:42`), which passes `None` as the system's appearance to `ThemeRegistry::resolve_active` (`theme/resolver.rs:105`), and `auto` with no system appearance is dark. Only cargo-port observes the system: `spawn_appearance_poller` (`theme/poller.rs:180`) is called at `crates/cargo-port/src/tui/terminal/run.rs:202` and applied by `apply_os_appearance` (`crates/cargo-port/src/tui/app/async_tasks/config.rs`). So cargo-tile and cargo-handler always draw the dark theme. With transparent on, the dark theme's white default text (`theme/fallback.rs:65`) stands on the terminal's own light background: the memory total (`SummaryMemoryTotal::foot`, `crates/cargo-tile/src/render.rs:219`). The light theme's default text is black (`fallback.rs:109`).

Rule:
1. `tui_pane` remembers the system's appearance beside the active theme, as a named state and never a bare `Option`: not yet observed, or observed light or dark. `resolve_appearance` reads it, so `install_theme` and `apply_settings` both resolve `auto` from it. A pinned `light` or `dark` ignores it, as today. Not yet observed resolves dark, as today.
2. One public function in `tui_pane` takes a newly observed appearance and the app's `AppearanceConfig`, stores the appearance and makes the theme it now selects the active one (`set_active_theme`, `theme/state.rs:141`). Model it on cargo-port's `apply_os_appearance`; cargo-port's own code is not this unit's and does not change.
3. Each app reads the system's appearance once before its first frame, so the first frame is already right. Reuse what the poller reads (`dark_light::detect` off Linux, the settings portal on Linux); when it cannot be read, the app starts dark, as today.
4. Each app follows a change while it runs. cargo-tile and cargo-handler have no tokio runtime, and `tui_pane` has one (`rt`, `time`): add a public `tui_pane` function that runs the existing poller task on its own named thread with a current-thread runtime and calls `on_change` there. The app's callback only sends on a channel; its loop (`crates/cargo-tile/src/terminal.rs`, the `try_recv` drains near 231 to 261; `crates/cargo-handler/src/terminal.rs:67`) applies the change through rule 2 and redraws. Cost, to state in the changelog entry's wording only if it is user-visible: one idle thread; on Linux one held session-bus subscription and no polling; elsewhere one `dark_light::detect` every 1500 ms (`POLL_INTERVAL`, `theme/constants.rs:69`), as cargo-port already pays.
5. With the light theme active, read every ink the summary and the cells use, transparent on and off: a literal `Color::White` or `Color::Black`, and each blend toward a ground (`blend_color` at `render.rs:961`, `Layout::ink` at 1476, `pane_background` at `pane/chrome.rs:157`). Each must read on the light theme's ground and on a light terminal's own background. Fix what does not, in the theme or at the call site, and list each in the summary.

Tests, all pure; none starts the watcher thread or reads the system:
- `tui_pane`: `auto_resolves_from_the_observed_system_appearance`, `a_pinned_mode_ignores_the_system`, `an_unobserved_system_resolves_dark`, `a_newly_observed_appearance_changes_the_active_theme`.
- `cargo-tile` `render.rs`: `the_summary_memory_total_reads_on_the_light_theme` — with the light theme active the value's foreground is the light theme's default text and differs from its ground, transparent on and off.

Docs: `crates/cargo-tile/README.md` line 77 says `auto` follows the system's light or dark setting; check `crates/cargo-handler/README.md` says the same where it names the mode. One Fixed line under `## [Unreleased]` in each of the three changelogs.

**Files:**
- `crates/tui_pane/src/theme/state.rs`, `crates/tui_pane/src/theme/resolver.rs`, `crates/tui_pane/src/theme/poller.rs`, `crates/tui_pane/src/theme/mod.rs` — the remembered appearance, the thread that watches it, tests.
- `crates/tui_pane/src/app_config/theme_install.rs`, `crates/tui_pane/src/app_settings/step.rs` — `resolve_appearance` reads the remembered appearance.
- `crates/tui_pane/src/lib.rs` — the two new public functions.
- `crates/tui_pane/CHANGELOG.md` — Fixed line.
- `crates/cargo-tile/src/terminal.rs`, `crates/cargo-tile/src/app.rs` — the startup read, the channel, applying a change.
- `crates/cargo-tile/src/render.rs` — the test, any ink rule 5 finds.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md` — the mode's wording, Fixed line.
- `crates/cargo-handler/src/terminal.rs`, `crates/cargo-handler/src/app.rs` — the same wiring.
- `crates/cargo-handler/README.md`, `crates/cargo-handler/CHANGELOG.md` — the mode's wording, Fixed line.

**Seats:** 2 writers — the framework and one app, and the other app.
- `impl` — every `crates/tui_pane` file above and every `crates/cargo-handler` file above. Its first edit adds the two public functions with their final signatures and posts them on the board.
- `test` — opens as impl: every `crates/cargo-tile` file above.

**Constraints from prior phases:** Phases 1 to 3 changed only `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/tiles/` and changelogs; nothing in `theme/`, `app_config/` or either app's `terminal.rs`. `text_default()` (`theme/accessors.rs:100`) stays the theme's colour: painted surfaces such as the status bar (`bar/palette.rs:69`) pair it with a painted ground. New public items are the two functions and, if a caller outside `tui_pane` must name it, the remembered-appearance type; name each in the summary. No `#[allow]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures of cargo-tile beside a real `cargo check`, 126x50, six ways: `mode = "light"` on a light terminal and `mode = "dark"` on a dark one, each with transparent off and on; and `mode = "auto"` with transparent on and off, whose theme matches this machine's system setting, named in the notice. Every value in the summary and the cells reads in all six; the memory total is named in the notice.
- A fresh helper's design check of those six shots: pass.

### Phase 5 — A floor for cells, and the summary alone below it  · status: todo

#### Work Order

**Goal:** cargo-tile never makes a command cell narrower than 40 cells, a window too small for the grid shows the summary alone, filling the window, and what a narrow window still shows (the summary, its border title, its foot readout and the status line) is whole, shortened with `…`, or absent at every width.

**Spec:**

1. **Status line fit** (`tui_pane`, `status_line.rs`, `render` at 162 and `render_sections` at 272). Nothing is measured today: the right block is right-aligned, starts at column 0 when it is wider than the row, and its tail (`? shortcuts`) is cut. New order, whole items only:
   1. the globals (` ? shortcuts`) are kept while they fit the row with their one trailing cell;
   2. notes are added in front of the globals from the last note backwards while each fits whole — so the app's name and version, the first note, go first;
   3. the left segment (uptime, then navigation) is drawn only when it fits whole in the room left of the right block with one cell between; its navigation spans go before the uptime does;
   4. the centre is drawn only when it fits whole between them.

   Add a test module to `status_line.rs`; build the spans with the file's own helpers.
2. **A floor where cells stop being useful** (user, 2026-10-08: "cargo tile becomes useless when it is too small"). The framework minimum (`MIN_TILE_WIDTH`, 8) stays as it is, because `cargo-handler` draws its agent cells on the same grid and its narrow cells are its own design. Add `TileGrid::set_min_tile_width(&mut self, width: u16)` in `tui_pane` (it writes `settings.min_tile_width`, never below the framework minimum), and have cargo-tile call it once where it builds its grid (`app.rs:207`) with a new `MIN_CELL_WIDTH = 40`: 38 cells inside hold a whole `[account] project` heading of ordinary length, a pid with a 28-cell command, and the whole foot readout; an 80-column terminal holds two columns. `fits` (`grid.rs:1777`) already refuses to open a cell that would make columns narrower — on `+`, on a command arriving, on `sync` — so in cargo-tile five columns need 196 terminal columns, four 157, three 118, two 79, one 40. A command refused a cell is still counted in the summary, as today.
3. **Too small shows the summary alone** (user, 2026-10-08: "the only way it is useful at narrow column width is just simply to show the summary table only"). Add `TileGrid::holds_in(&self, area: Rect, growth: TileGrowth) -> bool`, crate-private (`pub(super)`): the `fits` test applied to every arrangement the grid may still draw, which is the one on screen (`slots` at `depth`), the start of the transition in flight (`transition.held`), and each queued step through `target()` (822). The target alone is not enough: `drawing_at` (1149) draws from the transition's start, so a two-column target fits 79 columns while the three-column arrangement it is closing from is still drawn, under 40 a cell. For each it counts every position laid out, cells `+ TABLE_CELL + depth.saturating_sub(1)`, because `Grid::new` adds the summary's depth before it divides the columns; `fits` given the cell count alone accepts a deep-summary grid that does not fit. In `draw_tile_grid` (`draw.rs:144`), after `grid.sync`, when it is false and contents are shown: draw the summary cell alone, its frame on the whole `area`, through the same path a settled summary cell takes in `draw_placements` and `draw_cell` (title, labels, foot, readout, ground and tint as any cell), with no column band and no command cell, and return. The summary's demand and its measured width come from the whole area's inner width in that frame, not from the arrangement's column. There is no motion into or out of this state: the frame after a resize is the summary alone or the settled grid. The grid keeps syncing and keeps its cells, so they stand in their settled rects in the first frame whose area holds them; a command refused a cell while the window was small opens one by the ordinary `sync` rule once there is room. Contents hidden: nothing is drawn, as today. An area too small for a frame with one body row and one body cell draws nothing. The same rule serves `cargo-handler` at its own 8-cell minimum, where today it draws border-only slivers. This replaces the earlier plan's `window too small` notice: no `TILE_GRID_TOO_SMALL` constant and no surface type are added.

   The state is one thing, read by everything: a crate-private `GridDisplay::{Cells, SummaryAlone}` (name the seat's), worked out from `holds_in` over the last area and growth, serves drawing, hit-testing and focus. Today a draw-only branch would leave `cell_at` (1108) dividing the area as the hidden grid and Tab (`cycle_focus`, 1057, through `host.rs:56`) walking cells nobody sees. While the summary is alone: `cell_at` answers the summary for every point in the area; `cycle_focus` and the arrow steps take no step and report the key unspent; the summary is drawn as the focused cell. The focus the grid holds is not changed, so the cell that had it has it again in the first frame that shows the cells.
4. **The summary reads at every width** (`cargo-tile`, `draw_summary`, `render.rs:611`). Alone in a narrow window the summary is all the user has. Its table already sheds columns (`TABLE_NO_COLUMNS_MARKER`, `narrow_table_buffer` at `render.rs:4801`). Pin it: at every inner width from 6 to 38, each header, each table value and the foot is whole, shortened with `…`, or absent. Fix in `draw_summary` whatever the test finds cut bare, by the plan's first invariant (the end of a path survives; a value is never cut without its mark).
5. **Border title** (`tui_pane`, `write_overlay`, `frame.rs:661`). A title wider than its row is cut bare (` summar`, `┌ Tar┐`). For `OverlayStyle::Title`, when the text is wider than `row.width`, draw its first `row.width - 1` cells and then `…`; a row one cell wide draws `…` alone. `written_row` already records the drawn width. Labels (`OverlayStyle::Label`) are whole or absent through `clear_run` and do not change. Name the glyph `TITLE_ELISION` in `crates/tui_pane/src/pane/constants.rs`. `a_title_stops_inside_the_corner` now expects `┌ Ta…┐`. Measure with `unicode_width`, as the file does.
6. **Foot readout** (`tui_pane`, `draw.rs`). `readout_area` clamps the line to the room and the `Paragraph` cuts it (`conten`, `r/c: 22/2`). `rows_readout_line` becomes `rows_readout_lines(inner, rows, measured_at) -> Vec<Line<'static>>`, widest first:
   1. the whole line as today: `content rows: N[ @ W]  r/c: H/W`;
   2. `content rows: N @ W` — only when `measured_at != inner.width`;
   3. `content rows: N`.

   `draw_rows_readout` draws the first candidate whose width is at most `inner.width - TILE_ROWS_RIGHT_INSET`, right-aligned as now, and nothing when none fits. `draw_rows_readout_after_foot` runs the same list against the room left after the summary foot. The readout row stays reserved either way, so no cell changes height. One exception, from Phase 2's last design check: a cell with a single body row drew `content rows: 0  r/c: 1/49` and no label, so it had no name. A cell with one body row gives that row to its contents (its label) and draws no readout; `readout_row` returns `CellTooSmall` there. Today that reservation rests on the clamp: `content_area` (`draw.rs:499`) asks `readout_area(inner, inner.width) -> Option<Rect>` (517), which clamps the width to the room. So separate the two: replace `readout_area` with `readout_row(inner) -> ReadoutRow`, `ReadoutRow::{Reserved(Rect), CellTooSmall}`, the whole last interior row inside the right inset. `content_area` reads only that. The drawing functions pick the first candidate no wider than the reserved row's room and right-align it there; no candidate is ever clamped, and no readout function returns a bare `Option<Rect>`.

Tests, all pure: `status_line.rs` — at every width from 0 to the full row's natural width, each item's text appears whole or not at all, and the globals outlast the notes. `grid.rs`/`draw.rs` — with the minimum set to 40, `fits` holds at 196, 157, 118, 79 and 40 columns and fails one under each; a grid left at the framework minimum still fits three columns in 22; `holds_in_counts_the_headed_summary_depth`; `a_grid_too_wide_for_its_window_draws_the_summary_alone` (three columns opened at 200 columns, drawn at 100: the summary's frame is the whole area, its title on the top row, no column divider and no command cell's text anywhere); `the_cells_stand_settled_in_the_first_frame_that_holds_them` (the same grid drawn again at 200: every cell in its settled rect, nothing in flight); `a_command_refused_in_a_small_window_opens_when_there_is_room` (first synced at 30 columns with two commands, then at 100); `a_hidden_grid_too_small_draws_nothing`; `a_closing_column_in_flight_never_draws_a_cell_under_the_floor` and `a_depth_change_in_flight_never_draws_a_cell_under_the_floor` (every snapshot of the step through `drawing_at`, at a width the target fits and the start does not); `a_click_anywhere_in_the_summary_alone_picks_the_summary`; `tab_takes_no_step_while_the_summary_is_alone` (`host.rs`, beside `tab_walks_the_cells_and_wraps`); `focus_returns_to_its_cell_with_the_cells`. `render.rs` — `the_summary_alone_is_whole_marked_or_absent_at_every_width` (item 4).

- `frame.rs`: the updated `a_title_stops_inside_the_corner`, and `a_title_one_cell_wide_is_the_mark_alone`.
- `draw.rs`: `the_rows_readout_is_a_whole_candidate_at_every_width` — for each inner width from 0 to 60: the readout row, trimmed, is empty or equals one of the candidates exactly. Update `draw_tile_cell_draws_only_the_readout_on_its_foot_row` (1443; a four-row cell) only if its expected text changes (it should not). `a_cell_with_one_body_row_draws_its_contents_and_no_readout` — one interior row: the row holds the cell's contents and no readout text.

README: the `[tiles]` section of `crates/cargo-tile/README.md` gains two sentences: command cells are never narrower than 40 cells, and a window too small for them shows the summary alone. Changelogs: Changed lines under `## [Unreleased]`.

**Files:**
- `crates/tui_pane/src/bar/status_line.rs` — fit pass, test module.
- `crates/tui_pane/src/tiles/grid.rs` — `set_min_tile_width`, `holds_in`, the display state, `cell_at`, `cycle_focus`, tests.
- `crates/tui_pane/src/tiles/host.rs` — keys and clicks while the summary is alone, tests.
- `crates/tui_pane/src/tiles/draw.rs` — the summary alone in `draw_tile_grid`, readout candidates, tests.
- `crates/tui_pane/src/pane/frame.rs` — `write_overlay`, tests.
- `crates/tui_pane/src/pane/constants.rs` — `TITLE_ELISION`.
- `crates/tui_pane/CHANGELOG.md` — Changed lines.
- `crates/cargo-tile/src/render.rs` — `draw_summary`, test.
- `crates/cargo-tile/src/constants.rs` — `MIN_CELL_WIDTH`.
- `crates/cargo-tile/src/app.rs` — sets the grid's minimum cell width.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md` — the floor and the summary alone.

**Seats:** 2 writers — the grid and the status line, and everything else.
- `impl` — `crates/tui_pane/src/bar/status_line.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/host.rs`, `crates/tui_pane/CHANGELOG.md`. Its first edit adds `TileGrid::set_min_tile_width` with its final signature and posts it on the board.
- `test` — opens as impl: `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/pane/constants.rs`, `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md`. It messages `impl` for its line in the `tui_pane` changelog.

**Constraints from prior phases:** Phase 3 made `TileDrawing` hold `pieces: Vec<TilePiece<Id>>` (a `TilePlacement` and its `PieceName::{Shown, OnTheOtherPiece}`) and `column_bands`; `draw_placements` draws a cell's border title or empty-cell number only on the piece whose name is `Shown`, and `piece_name` in `grid.rs` is the one place that decides it. A cell drawn alone has one piece, always `Shown`. Phase 1 added `draw_placements` under `draw_tile_grid` in `draw.rs`; the summary-alone branch goes in `draw_tile_grid` (`draw.rs:144`) after `grid.sync` and before `grid.drawing` and `draw_placements(buffer, area, &drawing, &demands, &widths, contents, cells)`, which stays the owner of the ordinary ground and frame. Phase 2 changed how `grid.rs` places moving pieces and column bands and added `TileGrid::drawing_at`; it changed nothing a settled grid shows. Phase 3 gave each piece in `TileDrawing` a title role: the summary drawn alone shows its title. Phase 4 made `auto` follow the system's appearance. `write_overlay` (`frame.rs:661`) still cuts a title bare and `readout_area` still clamps the readout: items 5 and 6 fix both here, because the summary alone draws both in a narrow window. `set_min_tile_width` is this phase's only new public item; no `#[allow]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures beside a real `cargo check`, 50 rows: cells at 200, 126, 90, 64, 48 and 40 columns; the summary alone at 39, 32, 24 and 16 columns; and one window shrunk from 200 to 100 while three columns are open (the summary alone), then grown back to 200 (the cells back). In the summary alone nothing is cut bare. At every width the summary, each border title, each foot readout and the status line are whole or marked, and no command cell is drawn under 40 wide. Directory headings and ancestry inside command cells are Phase 6's and do not fail this gate.
- A fresh helper's design check of all twelve shots: pass. The notice lists the shots on each side of the switch so the user can judge the width.

### Phase 6 — Headings and ancestry are whole or marked  · status: todo

#### Work Order

**Goal:** Inside a command cell, a directory heading is drawn whole, shortened with `…`, or left out, and ancestry text breaks at path and word boundaries or ends in `…` — never cut bare.

**Spec:**

1. **Directory heading** (`cargo-tile`, `draw_path_group`, `render.rs:1634`). Today `PathGroup::heading()` is one span at natural width in an unwrapped `Paragraph`, cut at the cell edge (`[natep`). Add `PathGroup::fitted_heading(&self, room: u16) -> String` and draw that; `room` is `area.width` less `SECTION_HEADER_INDENT`. It returns the first of these that fits `room` (measured with `cell_width`):
   1. the whole heading, `[account] path` (or `path` alone for an unqualified group);
   2. `[account] …/rest`, dropping whole leading path components, keeping as many trailing components as fit;
   3. `[account] …tail`, the last component cut from its head, while at least `HEADING_MIN_TAIL` (8) cells of it show;
   4. forms 2 then 3 again without the `[account] ` prefix;
   5. the empty string.

   A shorter `…tail` form is not built: the floor keeps every command cell at 40 cells or wider, where form 4 always fits.

   The project name is the end of the path, so the end is what survives. `…` is `ANCESTRY_ELISION`'s glyph; name a `HEADING_ELISION` constant for it. `heading_gauge` and `summary_width` keep measuring the whole heading: the gauge already goes before the heading sheds anything, and `widen_summary` asks for the whole width.
2. **Wrap at boundaries** (`cargo-tile`, `Wrap::push`, `wrap.rs:72`). A word wider than a whole line is cut today at a raw cell count. New rule for that branch: break after a boundary character — any of `WRAP_BREAK_AFTER = "/-=_.:,"` — taking the last boundary that fits the room left on the current line; with none in that room and the line not empty, wrap first and try a whole line; with none in a whole line either, cut at the line's width as today. Words that fit a line are untouched. `wrapped` serves the ancestry chain and the table's command column, and demand and draw both call it, so they stay in step.
3. **A cut level says so** (`draw_ancestry`, `render.rs:983`). `lines.truncate(budget.max(1))` drops a level's last lines bare. When it drops any, the last kept line ends in `ANCESTRY_ELISION`: appended when the line has a free cell inside `ancestry_room`, otherwise replacing its last cell.
4. **No room, no command** (`ancestry_stem`, `ancestry_room`, `render.rs:1130`, `1144`). When a level's room for its command is under `ANCESTRY_MIN_COMMAND_WIDTH` (8), the level draws its pid alone on one row; `ancestry_rows` counts it as one row. The pid stays whole or absent as now.

Tests, all pure:
- `render.rs`: `every_heading_width_is_whole_marked_or_absent` — for each width from 0 to the whole heading's width, for a qualified and an unqualified group: the drawn heading is the whole heading, or contains exactly one `…` and ends with a suffix of the path, or is empty; and it never exceeds the width. One case pins `[natepiano] …/tool-based-ui-frame-time`.
- `wrap.rs`: a long path breaks only after `/`, a flag after `=` and `-`, a run with no boundary still cuts at the width; update the tests that pin today's cuts (`wrap.rs` 219, 227; `render.rs` 3112, 3140).
- `render.rs`: a truncated level ends in `…`; a level under the minimum room draws its pid alone; demand equals draw at widths 22 to 60.

Changelog: one Changed line under `## [Unreleased]` in `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/cargo-tile/src/render.rs` — `PathGroup::fitted_heading`, `draw_path_group`, `draw_ancestry`, `ancestry_stem`, `ancestry_room`, `ancestry_rows`, `ancestry_lines`, tests.
- `crates/cargo-tile/src/wrap.rs` — boundary breaks, tests.
- `crates/cargo-tile/src/constants.rs` — `HEADING_ELISION`, `HEADING_MIN_TAIL`, `WRAP_BREAK_AFTER`, `ANCESTRY_MIN_COMMAND_WIDTH`.
- `crates/cargo-tile/CHANGELOG.md` — Changed line.

**Seats:** 2 writers — drawing in `render.rs`, wrapping in `wrap.rs`.
- `impl` — `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/CHANGELOG.md`. Its first edit adds the four constants and posts their names on the board.
- `test` — opens as impl: `crates/cargo-tile/src/wrap.rs`.

**Constraints from prior phases:** Phases 1 to 4 changed `tui_pane` and no rule for text inside a cargo-tile cell. Phase 5 set cargo-tile's floor (`MIN_CELL_WIDTH = 40`), so every command cell drawn here is 40 cells or wider; it added `TITLE_ELISION` in `tui_pane`, made the foot readout a list of whole candidates, and fixed in `draw_summary` whatever was cut bare. None of that changes here. `wrapped` also serves the summary table's command column, so Phase 5's `the_summary_alone_is_whole_marked_or_absent_at_every_width` passes unchanged. No `tui_pane` change; no new public item; no `#[allow]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint cargo-tile` once, then `... test cargo-tile` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures beside a real `cargo check`, 50 rows, at 200, 126, 90, 64, 48 and 40 columns: no heading is cut bare, and ancestry breaks only at a boundary or ends in `…`.
- A fresh helper's design check of those six shots: pass.

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

**Constraints from prior phases:** Phases 1 to 6 added render tests to `tui_pane` and `cargo-tile`; each gate asked for under a second, and this phase's line is 0.8 s, so the six measurements decide whether any of them joins the slow set.

**Acceptance gate:**
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, the source-switch reader is first, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.

### Phase 8 — The summary takes the rows it asks for, not a second position  · status: todo

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

**Constraints from prior phases:** Phase 2 made `TileGrid::drawing_at(area, growth, raw)` the pure entry to a motion and made `queue_with_depth` keep every surviving cell in a column it starts or ends the queue in; neither changes here. Phase 3 made a column the step removes close as one piece (`BandPieceMotion::ClosingWithColumn`; test `no_piece_in_a_closing_column_changes_height`, with and without a widened summary over the closing column) and gave each piece a name role (`TilePiece`, `PieceName`, `piece_name`); a change to the summary's rows keeps both, and the `a_title_is_on_screen_once_*` and `an_empty_cell_number_is_on_screen_once_*` render tests pass unchanged. Phase 5 added `TileGrid::set_min_tile_width` and the crate-private `holds_in`, which checks every arrangement still to be drawn, each at its own depth: a summary that stays shallower lets a grid hold in a smaller window, it must never let a wider arrangement still in flight be drawn under the floor, and `holds_in`'s tests pass unchanged. `cargo-handler` draws on the same grid, so its summary follows the same rule. No new public item; no `#[allow]`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` once, then `... test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` green; the named tests pass; no test in the run takes a second.
- Unit director's capture at 200x50 with seven `+` and seven `-` (it reaches ten cells and four columns), transparent off: in every settled frame the summary shows no blank body row while a cell in its column is short of what it asks, and Phase 2's motion measures still read zero.
- A fresh helper's design check of the settled shots and one motion shot per step, against Phase 2's shots of the same steps: pass.
