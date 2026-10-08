# tile-fixes

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** cargo-tile draws a moving cell on the painted ground, keeps every label and readout whole or marked at narrow widths, and nextest starts the slow tests first.

> **Production: remove-running-section** — unit `tile-fixes-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-08 06:40 PDT

now that we've added transparent - when a cell is being resized you can see (in this case with transparent off) something is happening that causes an artifact of making it looks like an oversized border around the resizing (probably removing) cell in cargo tile - i haven't seen this before and it looks odd

2026-10-08 06:5x PDT (relayed by the showrunner)

do the narrow column work - and for now just make sure slow tests run first in a cargo nextest run

## Delegation Context

- **Project:** `cargo-tile` — a terminal grid of every running cargo command, one cell per command plus a summary cell. Its grid, borders and status line are drawn by `tui_pane`, which `cargo-handler` and `cargo-port` also build on.
- **Worktree:** `/home/natepiano/rust/cargo-liner-tile-fixes`, branch `remove-running-section-tile-fixes`. Every seat works only here. Set by the showrunner (production doc, Units table).
- **Stack:** Rust workspace, ratatui 0.30.2, cargo-nextest (0.9.143 on natedev, 0.9.136 on the Mac).
- **Layout:**
  - `crates/tui_pane/src/pane/` — `frame.rs` (shared borders, `draw_clipped`, titles), `chrome.rs` (tint and screen ground).
  - `crates/tui_pane/src/tiles/` — `draw.rs` (the grid's drawing and the foot readout), `grid.rs` (arrangement, motion, `fits`), `constants.rs`.
  - `crates/tui_pane/src/bar/status_line.rs` — the status line every app draws.
  - `crates/cargo-tile/src/` — `render.rs` (cells, headings, ancestry, status line notes), `wrap.rs`, `constants.rs`.
  - `.config/nextest.toml` — test ordering, groups and timeouts for the workspace.
- **Key files:**
  - `crates/tui_pane/src/pane/frame.rs` — `PaneFrame` (64), `draw_clipped` (193), `fill_pane` (222), `blit` (230), `GridLines::add_titled` (449), `overlay_row` (568), `written_row` (611), `write_overlay` (630), tests from 729 (`a_title_stops_inside_the_corner` 999, `a_shifted_pane_keeps_only_what_lands_in_its_clip` 1060).
  - `crates/tui_pane/src/pane/chrome.rs` — `pane_fill` (129), `screen_ground` (145), `pane_background` (157).
  - `crates/tui_pane/src/tiles/draw.rs` — `draw_tile_grid` (140), `draw_cell` (257), `draw_rows_readout` (348), `draw_rows_readout_after_foot` (354), `rows_readout_line` (375), `draw_rows_readout_line` (408), `draw_summary_foot` (416), `readout_area` (460), tests from 488 (`StubCells` 495, `settled_grid` 548).
  - `crates/tui_pane/src/tiles/grid.rs` — `TileGrid` (315), `sync` (541), `placements` (1045), `columns` (1472), `fits` (1625), `shared_run` (1732), `wrapping_cell` (1897), `column_band` (1955).
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
  - Every literal the code names lives in its crate's `constants.rs` (existing crate convention).
  - No test takes a second. Render tests are pure and run on buffers; none waits on the clock (user rule, memory `no-multi-second-tests`).
  - A `tui_pane` change reaches `cargo-handler` and `cargo-port`; a gate that touches `tui_pane` also runs `verify.sh test` for both (production doc, Merge tests). No existing `tui_pane` public signature changes, and `cargo-handler`'s cell sizes do not change (author's calls).
  - Format with `cargo +nightly fmt` only (user rule, memory `rustfmt-nightly-only`).
  - No UX guide covers cargo-tile (production doc, Production rules): design checks judge by the three gods in `~/.claude/docs/decision_criteria.md`.
  - Captures are the unit director's, taken before each checkpoint notice with a design check by a fresh helper (showrunner's gate). Screens are pty captures of the real `target/debug/cargo-tile tile`, replayed with pyte and split into frames on `\x1b[?25l`; the settled screen is the last completed frame, never a byte-offset sample.

## Phases

### Phase 1 — A cell in flight stands on the painted ground  · status: todo

#### Work Order

**Goal:** With `transparent = false`, a cell crossing columns is painted exactly like a cell standing still: no ring of unpainted cells around it, and none of its text on a border row.

**Spec:**

Reproduced 2026-10-08: seven groups in a 100x36 area, remove the first. Each later cell crosses a column as two `PaneFrame::shifted` pieces (`wrapping_cell`, `grid.rs:1944`) for the 720 ms step, and 91 to 93 cells around the two pieces carry `bg == Color::Reset`. Settled frames carry none. With `transparent = true` nothing is painted anywhere, so nothing shows.

Cause: `draw_tile_grid` paints the area with `screen_ground()` (`draw.rs:156`), and a still pane draws straight onto that. A pane in flight draws into `Buffer::empty(frame.rect)`; `fill_pane` tints only `inner`; `blit` copies the whole rect, so the ring of border cells arrives with a reset background and replaces the painted ground. `GridLines::render` then sets only symbol and foreground there.

1. **Ground in the scratch buffer.** In `draw_clipped`'s moving path, after `Buffer::empty(frame.rect)` and before `fill_pane`, lay `chrome::screen_ground()` over the whole of `frame.rect` when it is `Some`: `scratch.set_style(frame.rect, Style::default().bg(ground))`. A piece in flight then carries what a still cell has: ring on the ground, tint inside. With the screen transparent `screen_ground()` is `None` and the path is unchanged. Update the doc comments on `draw_clipped` and `fill_pane`, which still say the border keeps the terminal's own background.
2. **Contents stay off the clip's border rows.** `blit` copies a piece's rows onto every row of `frame.clip`, and the clip is a column band that includes the column's own top and bottom border rows (`column_band`, `grid.rs:1955`), so a piece leaving over the top writes its text onto that border row, where no line is redrawn. In `blit`, a source row that is an interior row of the piece (`rect.top() < source < rect.bottom() - 1`) is not copied onto `clip.top()` or `clip.bottom() - 1`. The piece's own ring rows are still copied there, so the ground under the line is painted.
3. **Placements handed in, for the grid test.** Move the drawing half of `draw_tile_grid` — everything after `let placements = grid.placements(area, growth);` — into a private `fn draw_placements<Id: Clone + Eq + Debug>(buffer, area, placements: &[TilePlacement<Id>], demands: &TileDemands<Id>, widths: &[(TileContent<Id>, u16)], contents: TileGridContents, cells: &impl TileCells<Id>)`. `draw_tile_grid` calls it. No behavior change; it lets a test hand in a piece in flight without waiting on the clock.

Tests, all pure:
- `frame.rs`: `a_shifted_pane_lands_on_the_screen_ground` — `set_transparent_background(false)`, paint a target buffer with `screen_ground()`, `draw_clipped` a `PaneFrame::shifted` piece; every cell of the piece inside its clip has the ground as background on its ring and `pane_background(false)` inside, none `Color::Reset`. A second case with `set_transparent_background(true)`: every cell keeps `Color::Reset`.
- `frame.rs`: `a_shifted_pane_keeps_its_contents_off_the_clip_border_rows` — a piece shifted so an interior row lands on `clip.top()`: that row holds no symbol the piece drew.
- `draw.rs`: `a_piece_in_flight_leaves_no_cell_unpainted` — `draw_placements` with the summary still and one group as two shifted pieces, transparent off: no cell of `area` has `bg == Color::Reset`.

Changelog: one line under `## [Unreleased]` → Fixed in `crates/tui_pane/CHANGELOG.md` and `crates/cargo-tile/CHANGELOG.md`.

**Files:**
- `crates/tui_pane/src/pane/frame.rs` — `draw_clipped`, `blit`, doc comments, two tests.
- `crates/tui_pane/src/tiles/draw.rs` — `draw_placements` split, one test.
- `crates/tui_pane/CHANGELOG.md` — Fixed line.
- `crates/cargo-tile/CHANGELOG.md` — Fixed line.

**Seats:** 2 writers — the work splits by file inside `tui_pane`; both tests read private items, so they live in-module with the code they prove.
- `impl` — `crates/tui_pane/src/pane/frame.rs`, both changelogs.
- `test` — opens as impl: `crates/tui_pane/src/tiles/draw.rs` (the `draw_placements` split and its test, written against Spec item 1).

**Constraints from prior phases:**

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port` and `... lint tui_pane` green; the three named tests pass; no test in the run takes a second.
- Unit director's capture, `transparent = false`, 200x50: start `cargo-tile tile`, open five empty tiles with `+`, close them with `-`, replay every completed frame with pyte. No frame has a default-background cell inside the grid area, and no frame has a letter or digit on a row that is a border row in the settled frame before or after it. The same run with `transparent = true` still draws every frame.
- A recording of the same motion in Ghostty with `transparent = false`, and a fresh helper's design check of its frames: pass.

### Phase 2 — Headings, titles and readouts are whole or marked  · status: todo

#### Work Order

**Goal:** At every cell width a directory heading, a border title and the foot readout are drawn whole, shortened with `…`, or left out — never cut bare.

**Spec:**

1. **Directory heading** (`cargo-tile`, `draw_path_group`, `render.rs:1634`). Today `PathGroup::heading()` is one span at natural width in an unwrapped `Paragraph`, cut at the cell edge (`[natep`). Add `PathGroup::fitted_heading(&self, room: u16) -> String` and draw that; `room` is `area.width` less `SECTION_HEADER_INDENT`. It returns the first of these that fits `room` (measured with `cell_width`):
   1. the whole heading, `[account] path` (or `path` alone for an unqualified group);
   2. `[account] …/rest`, dropping whole leading path components, keeping as many trailing components as fit;
   3. `[account] …tail`, the last component cut from its head, while at least `HEADING_MIN_TAIL` (8) cells of it show;
   4. forms 2 then 3 again without the `[account] ` prefix;
   5. `…tail` with whatever fits, down to one cell of the name;
   6. the empty string.

   The project name is the end of the path, so the end is what survives. `…` is `ANCESTRY_ELISION`'s glyph; name a `HEADING_ELISION` constant for it. `heading_gauge` and `summary_width` keep measuring the whole heading: the gauge already goes before the heading sheds anything, and `widen_summary` asks for the whole width.
2. **Border title** (`tui_pane`, `write_overlay`, `frame.rs:630`). A title wider than its row is cut bare (` summar`, `┌ Tar┐`). For `OverlayStyle::Title`, when the text is wider than `row.width`, draw its first `row.width - 1` cells and then `…`; a row one cell wide draws `…` alone. `written_row` already records the drawn width. Labels (`OverlayStyle::Label`) are whole or absent through `clear_run` and do not change. Name the glyph `TITLE_ELISION` in `crates/tui_pane/src/pane/constants.rs`. `a_title_stops_inside_the_corner` now expects `┌ Ta…┐`. Measure with `unicode_width`, as the file does.
3. **Foot readout** (`tui_pane`, `draw.rs`). `readout_area` clamps the line to the room and the `Paragraph` cuts it (`conten`, `r/c: 22/2`). `rows_readout_line` becomes `rows_readout_lines(inner, rows, measured_at) -> Vec<Line<'static>>`, widest first:
   1. the whole line as today: `content rows: N[ @ W]  r/c: H/W`;
   2. `content rows: N @ W` — only when `measured_at != inner.width`;
   3. `content rows: N`.

   `draw_rows_readout` draws the first candidate whose width is at most `inner.width - TILE_ROWS_RIGHT_INSET`, right-aligned as now, and nothing when none fits. `draw_rows_readout_after_foot` runs the same list against the room left after the summary foot. The readout row stays reserved either way, so no cell changes height. `readout_area` no longer clamps a width it is handed.

Tests, all pure:
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
- `crates/tui_pane/CHANGELOG.md` — Changed line.

**Seats:** 2 writers — the work splits by crate.
- `impl` — `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/src/pane/constants.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/CHANGELOG.md`.
- `test` — opens as impl: `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/CHANGELOG.md`.

**Constraints from prior phases:** Phase 1 split `draw_tile_grid` into itself plus a private `draw_placements` in `draw.rs`, and changed `draw_clipped` and `blit` in `frame.rs`; neither touches titles or readouts.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port`, `... lint tui_pane` and `... lint cargo-tile` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures beside a real `cargo check`, 50 rows, at 200, 126, 90, 64, 30 and 24 columns: no heading, title or foot readout is cut bare in any of them.
- A fresh helper's design check of those six captures. Headings, titles and readouts must pass; defects in ancestry text, the status line and tile geometry at 30 and 24 are recorded for Phase 3 and do not fail this gate.

### Phase 3 — Ancestry, status line and a floor for the grid  · status: todo

#### Work Order

**Goal:** Ancestry text breaks at path and word boundaries, the status line drops whole items as the row narrows, and cargo-tile's grid never makes a column narrower than 24 cells.

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
5. **Floor for cargo-tile's grid.** The framework minimum (`MIN_TILE_WIDTH`, 8) stays as it is, because `cargo-handler` draws its agent cells on the same grid and its narrow cells are its own design. Add `TileGrid::set_min_tile_width(&mut self, width: u16)` in `tui_pane` (it writes `settings.min_tile_width`, never below the framework minimum), and have cargo-tile call it once where it builds its grid (`app.rs:207`) with a new `MIN_CELL_WIDTH = 24` (22 cells inside: a whole pid, one gap and a 12-cell command; `content rows: N`; the summary title). `fits` (`grid.rs:1625`) already refuses to open a cell that would make columns narrower — on `+`, on a command arriving, on `sync` — and in cargo-tile that rule now holds at a readable width: three columns need 70 terminal columns, two need 47, one needs 24. A command refused a cell is still counted in the summary, as today.
6. **A window too small says so** (`tui_pane`). A terminal shrunk under a grid that is already open is not reflowed (author's call: no eviction, no change to `columns`). Add `TileGrid::holds_in(&self, area: Rect, growth: TileGrowth) -> bool`, the `fits` test applied to the arrangement the grid is headed for, and in `draw_tile_grid`, after `grid.sync`, when it is false: paint the ground as now, draw `TILE_GRID_TOO_SMALL` (`window too small`) centred on the area's middle row in `label_color()`, whole or not at all, and return. The grid keeps syncing, so it is right when the window grows back. `cargo-handler` gets the same notice at its own 8-cell minimum, where today it draws border-only slivers.

Tests, all pure: `wrap.rs` — a long path breaks only after `/`, a flag after `=` and `-`, a run with no boundary still cuts at the width; update the tests that pin today's cuts (`wrap.rs` 219, 227; `render.rs` 3112, 3140). `render.rs` — a truncated level ends in `…`; a level under the minimum room draws its pid alone; demand equals draw at widths 22 to 60. `status_line.rs` — at every width from 0 to the full row's natural width, each item's text appears whole or not at all, and the globals outlast the notes. `grid.rs`/`draw.rs` — with the minimum set to 24, `fits` holds at 70, 47 and 24 columns and fails one under each; a grid left at the framework minimum still fits three columns in 22; a grid opened at 100 columns and drawn at 40 draws only the notice; a 16-column area draws nothing.

README: the `[tiles]` section of `crates/cargo-tile/README.md` gains two sentences on the 24-cell floor and the notice. Changelogs: Changed lines under `## [Unreleased]`.

**Files:**
- `crates/cargo-tile/src/wrap.rs` — boundary breaks, tests.
- `crates/cargo-tile/src/render.rs` — `draw_ancestry`, `ancestry_stem`, `ancestry_room`, `ancestry_rows`, `ancestry_lines`, tests.
- `crates/cargo-tile/src/constants.rs` — `WRAP_BREAK_AFTER`, `ANCESTRY_MIN_COMMAND_WIDTH`, `MIN_CELL_WIDTH`.
- `crates/cargo-tile/src/app.rs` — sets the grid's minimum cell width.
- `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md` — floor and notice.
- `crates/tui_pane/src/bar/status_line.rs` — fit pass, test module.
- `crates/tui_pane/src/tiles/constants.rs` — `TILE_GRID_TOO_SMALL`.
- `crates/tui_pane/src/tiles/grid.rs` — `set_min_tile_width`, `holds_in`, tests.
- `crates/tui_pane/src/tiles/draw.rs` — the notice, test.
- `crates/tui_pane/CHANGELOG.md` — Changed lines.

**Seats:** 2 writers — the work splits by crate.
- `impl` — `crates/tui_pane/src/bar/status_line.rs`, `crates/tui_pane/src/tiles/constants.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/CHANGELOG.md`.
- `test` — opens as impl: `crates/cargo-tile/src/wrap.rs`, `crates/cargo-tile/src/render.rs`, `crates/cargo-tile/src/constants.rs`, `crates/cargo-tile/src/app.rs`, `crates/cargo-tile/README.md`, `crates/cargo-tile/CHANGELOG.md`.

**Constraints from prior phases:** Phase 1 added `draw_placements` under `draw_tile_grid` in `draw.rs`; the notice goes in `draw_tile_grid` before it is called. Phase 2 added `PathGroup::fitted_heading` and `HEADING_ELISION` in cargo-tile, `TITLE_ELISION` in `tui_pane/src/pane/constants.rs`, and made the foot readout a list of whole candidates (`rows_readout_lines`) — a cell 22 wide inside draws `content rows: N`.

**Acceptance gate:**
- `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`, `... test cargo-tile`, `... test cargo-handler`, `... test cargo-port`, `... lint tui_pane` and `... lint cargo-tile` green; the named tests pass; no test in the run takes a second.
- Unit director's settled captures beside a real `cargo check`, 50 rows, at 200, 126, 90, 64, 30 and 24 columns, plus one at 20 columns (the notice) and one with the window shrunk from 200 to 40 while three columns are open (the notice): nothing is cut bare anywhere, ancestry breaks only at a boundary or carries `…`, the status line holds only whole items.
- A fresh helper's design check of all eight captures: pass.

### Phase 4 — Slow tests start first  · status: todo

#### Work Order

**Goal:** In a full `cargo nextest run`, every test that takes 0.8 s or more on natedev or the Mac starts before any faster test. Ordering only: no test is rewritten (user's words: "for now").

**Spec:**

1. **Measure before.** On each machine run the suite CI runs, `cargo nextest run --all-features --workspace --exclude cargo-mend --tests`, three times with JUnit turned on from outside the repo config: a file `junit.toml` holding `[profile.default.junit]` / `path = "junit.xml"`, passed as `--tool-config-file tile-fixes:<path>/junit.toml`. Each `testcase` then carries its start `timestamp` and `time`. Record the wall time of each run and, per test, its slowest time. On the Mac work in a checkout of this branch outside `/tmp`, reached with `ssh mac`.
2. **The slow set** is every test at 0.8 s or more in any of the six runs, less the seven reader scenarios the config already orders. Known members to confirm by name with `cargo nextest list`: in `cargo-tile::unit_tests`, `progress::capture::…::incomplete_registration_inventory_sweeps_any_sampled_proven_pair`, eight in `shim_registration::wire::`, two in `hook::`; two each in `cargo-tile::shim_modes` and `cargo-tile::cli_lifecycle`; in `cargo-handler`, `census::codex::…::reads_threads_by_id_whoever_started_them` and `…::reads_interactive_threads_created_in_the_span`; in `tui_pane`, `attract::controller::tests::random_settings_corpus_reaches_every_variant_and_applies_every_draw`.
3. **One override.** Append to `.config/nextest.toml` one `[[profile.default.overrides]]` with `priority = 80` and a `filter` that names each slow test exactly: `binary_id(<id>) & (test(=<name>) | …)` per binary, joined with `|`. No regular expressions, so a test added later is not swept in by accident. A comment above it says what the list is, the threshold, the date measured, and how to re-measure. nextest resolves each setting from the first override that sets it, so the two existing priorities (100, 90), the `cargo-tile-readers` group and both `slow-timeout` entries keep working unchanged; the new override sets `priority` alone and names none of the reader scenarios.
4. **Measure after**, the same three runs per machine.

**Files:**
- `.config/nextest.toml` — one override and its comment. In scope for this unit by the showrunner's word (2026-10-08), though the Units row does not list it.

**Seats:** 1 writer — one file, nothing splits; `impl` holds it.
- `impl` — `.config/nextest.toml`.
- `test` — opens as impl: no file; runs the natedev measurements and hands the slow set to `impl`.

**Constraints from prior phases:** Phases 1 to 3 added render tests to `tui_pane` and `cargo-tile`; each runs well under a second and none joins the slow set.

**Acceptance gate:**
- `cargo nextest list` resolves every name in the new filter (no unmatched filter warning) on both machines.
- After the change, on each machine: sorted by start `timestamp`, the source-switch reader is first, and no test outside the reader group and the slow set starts before the last slow-set test starts. The reader group still runs one at a time, and a `cargo-tile::shim_modes` test still carries its 30 s `slow-timeout`.
- The unit director's notice reports, per machine, the median suite wall time before and after and the first thirty tests in start order.
