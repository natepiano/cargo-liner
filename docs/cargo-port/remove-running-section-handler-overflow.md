# handler-overflow

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** A tile whose content needs more rows than its column has continues at the top of the next column, titled `<name> · <role> · continued`; tiles after it move along, and short tiles take only the rows they need.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-15, the user (via the showrunner): "if a content row can't fit everything such as natedev with 42 rows - it should push hana down … natedev should show everything - cells that need to should be able to overflow into the next column - showing a - continued in the title - like "hana - showrunner - continue" … when we get to cells that don't need much they can give some of their rows back". Screenshots: the natedev tile wants 43 rows and gets 32; the hana tile wants 84 and gets 43; both are cut off.

## Delegation Context

- **Project:** `tui_pane` (the terminal UI framework whose `tiles` module lays out and animates the tile grid) and `cargo-handler` (the agent watcher, which draws its tiles through `tui_pane`).
- **Project started:** 2026-10-16T03:10:04.848+00:00
- **Stack:** Rust 2024, ratatui 0.30, cargo-nextest.
- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Layout:**
  - `crates/tui_pane/src/tiles/` — `mod.rs` (submodule list ≈17–23, re-exports ≈25–46), `grid.rs` (`TileGrid`, layout and motion), `draw.rs` (`TileCells`, `draw_tile_grid`, per-piece drawing and readouts), `growth.rs` (`TileGrowth`), `constants.rs`, `settings.rs`; new `flow_layout/` (phase 1: `mod.rs` with the `FlowLayout` anchor and its vocabulary, `split_builder.rs`, `tests.rs`).
  - `crates/tui_pane/src/lib.rs` — crate re-exports of `tiles::*` (≈497–518).
  - `crates/cargo-handler/src/` — `render.rs` (`draw_panes`, `Cells`), `agent_cell.rs`, `constants.rs`.
- **Key files** (line refs are this branch before tile-fixes merges; phases 2–4 re-read the files after the merge):
  - `crates/tui_pane/src/tiles/grid.rs` — `TileDemand` ≈85 and `TileDemands` ≈104 (`rows_for` ≈129); `TileContent` ≈147; `TilePlacement` ≈163; `PieceName` ≈174, `TilePiece` ≈183, `PieceHeightSnap` ≈194, `TileDrawing` ≈208; `HeldCellLayout` ≈1019; `Transition` ≈1048, `Step` ≈1068; `TileGrid` ≈1086 (fields ≈1087–1130); `resolved_grid` ≈1641, `count` ≈1753, `content_widths` ≈1761, `set_draw_layout` ≈1826, `tick` ≈1844, `sync_rendered_demands` ≈1887; `cycle_focus` ≈2706, `focus_cell` ≈2749, `cell_at` ≈2763, `holds_in` ≈2782, `display` ≈2804, `progress` ≈2814, `placements` ≈2827, `drawing`/`drawing_at` ≈2837/≈2880, `settled_drawing` ≈2857; `Grid` ≈3104 (`columns: Vec<Rect>` is each column's band); `columns` ≈3476, `fills` ≈3516, `shares` ≈3565, `shares_with_row_usage` ≈3576, `disjoint_demands` ≈3663 (a stacked piece costs framed − 1 rows, the column's last piece its full framed rows); `cell_wants` ≈3952; motion: `BandPieceMotion` (`Resident`, `Entering`, `Leaving`, `ClosingWithColumn`) ≈908, `moving_cell` ≈4076, `wrapping_cell` ≈4172, `column_bands` ≈4215, `place_band_pieces` ≈4358, `eased` ≈5271; `mod tests` ≈5293.
  - `crates/tui_pane/src/tiles/draw.rs` — `TileCells` ≈171 (`demands`, `rows_drawn`, `rows_kept`, `draw`, `group_title` ≈244 returning `Option<Line<'static>>`); `draw_tile_grid` ≈263; `draw_placements` ≈323 (per-piece loop ≈343–433, group title ≈420); `has_visible_body_row` ≈447; `draw_cell` ≈511; `add_readout_rows` ≈544; `measured_width` ≈563; `draw_rows_readout` ≈600 and `extent_readout_lines` ≈646 (count green while the rows fit above the readout, red once they do not); `content_area` ≈744; `rows_readout_height` ≈755; `readout_row` ≈764; `mod tests` ≈826.
  - `crates/tui_pane/src/tiles/constants.rs` — `MIN_TILE_HEIGHT` = 3 ≈21, `TILE_BORDER_ROWS` = 2 ≈29, `TILE_DEMAND_STEP` = 3 ≈34, `TILE_ROWS_READOUT_HEIGHT` = 1, `TILE_ROWS_RIGHT_INSET` = 1.
  - `crates/tui_pane/src/tiles/settings.rs` — `CellRowDemand` ≈16, `TileSettings` ≈51 (`animation_millis`), `demanded_rows` ≈95.
  - `crates/tui_pane/src/pane/frame.rs` — `draw_clipped` ≈196, `draw_at_destination` ≈230 (scratch `Buffer` pre-styled with `screen_ground` and `fill_pane`, then `blit` ≈274).
  - `crates/tui_pane/src/pane/chrome.rs` — `screen_ground` ≈145, `pane_background` ≈157, `pane_fill` ≈129.
  - `crates/tui_pane/CHANGELOG.md` — `## [Unreleased]`.
  - `crates/cargo-handler/src/render.rs` — `draw_panes` ≈87 calls `tui_pane::draw_tile_grid` ≈95 (the design doc named `terminal.rs`; the call is here); `Cells` ≈107 (its `account_column: AccountColumn` is decided once per frame in `Cells::new`); `impl TileCells<AgentCell> for Cells` ≈141; `group_title` ≈219 (name in `Role::Rainbow(entry.hue)`, then `entry.role_label.suffix()` in `label_color()`); `mod tests` ≈270 with fixtures `boss` ≈724, `drawn_buffer` ≈785, `drawn_buffer_in` ≈791 (also calls `draw_tile_grid` ≈799), `drawn_grid` ≈811.
  - `crates/cargo-handler/src/agent_cell.rs` — `CellView` ≈613, `CellView::fitted` ≈629 (compressed when the area is shorter than the full view), `height(entry, width, now, account_column)` ≈676, `draw(buffer, area, entry, cells, now, account_column)` ≈719, `draw_view` ≈739. A tile is its header lines and one tree table; there is no children table below it (`docs/cargo-handler/as-built/hierarchy.md`, Tile tree).
  - `crates/cargo-handler/src/census/placement.rs` — `RoleLabel::suffix` ≈91 (`" · <role>"` via `HEADING_SEPARATOR`).
  - `crates/cargo-handler/src/constants.rs` — `HEADING_SEPARATOR` = `" · "` ≈182.
  - `crates/cargo-tile/src/render.rs` — `draw_tile_grid` call ≈302, `impl TileCells<InvocationId> for Cells` ≈384; read only, it never calls the flow entry point.
- **Test lanes:** `tui_pane` has `crates/tui_pane/tests/`, but the flow code is crate-private, so its tests are unit-test files under `src/` declared with `#[cfg(test)] mod …;` (a file under a self-named module such as `draw.rs` or `render.rs` takes an explicit `#[path = "…"]`); `cargo-handler`: none (unit-test files under `src/`, the crate's pattern in `terminal.rs` ≈30 and `census/scan.rs` ≈22). A test file declared inside an existing `mod tests` block is a child of it and reads that block's private fixtures through `super::`.
- **Build:** `bash ~/.claude/scripts/direct/verify.sh check tui_pane`; `bash ~/.claude/scripts/direct/verify.sh check cargo-handler`
- **Test:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane`; `bash ~/.claude/scripts/direct/verify.sh test cargo-handler`; regression for the unchanged consumer: `bash ~/.claude/scripts/direct/verify.sh test cargo-tile`
- **Lint:** `bash ~/.claude/scripts/direct/verify.sh lint tui_pane`; `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler`
- **Invariants:**
  - Nothing on the UI thread runs a command or reads a file.
  - Tests run with nextest; format only with `cargo +nightly fmt`; no test takes a second.
  - Invalid states are unrepresentable: use types (checked constructors, enums, type state), never procedural guards (user rule 2026-10-15). No bare `Option` in new domain types.
  - Flow layout is opt-in by entry point and trait, never a flag (showrunner ruling 1): a consumer gets it only by calling `draw_tile_flow` with `cells: &impl FlowTileCells<Id>`. There is no boolean on `TileGrowth` or `TileGrid`.
  - cargo-tile behavior and snapshots stay unchanged: `draw_tile_grid` keeps its output byte for byte, and cargo-tile never calls the flow entry point.
  - Each frame measures each tile once: flow reads demand from the same per-frame `TileCells::demands` pass `draw_tile_grid` uses, never a second pass of its own.
  - Every new public `tui_pane` item gets a `crates/tui_pane/CHANGELOG.md` `## [Unreleased]` entry.
  - Coordination (production): tile-fixes (worktree `/home/natepiano/rust/cargo-liner-tile-fixes`) has large uncommitted changes in `tiles/grid.rs` and `tiles/draw.rs` — measured-once types `FrameCellRowAnswers`, `FrameWidthRowAnswers`, `FrameRowAnswer`, the provided method `TileCells::prepare_frame`, and `TileView::Auto` removed. No phase edits `grid.rs` or `draw.rs` before that work merges to main and this branch merges main.

## Phases

<!-- PHASE NUMBERING — binds every command that edits this doc.
     N is a bare integer, numbered from 1, contiguous, in execution order.
     Never a letter suffix (`4a`, `2b1a`) — those stop sorting and make ranges
     unreadable. Any spine edit (insert, split, merge, reorder, delete)
     resequences from the edit point to the end AND updates every cross-reference:
     substitute highest-first so `Phase 8` does not corrupt `Phase 12`, and
     re-check that each `Phases X–Y` range still spans the same set.
     Insert after the last `done` phase where possible — a done phase's number
     is baked into its checkpoint commit message and cannot be rewritten; if one
     must be renumbered, add an old→new mapping note to the doc.
     Full procedure: /plan:to_phased_plan → <PhaseNumbering/>. -->

### Phase 1 — tui_pane flow packer  · status: done

**Depends on:** none

#### As-built

- `FlowLayout::pack(bands: &[u16], tiles: Vec<TileRows<Id>>) -> FlowLayout<Id>` packs tiles in order, top to bottom and column by column, each at its framed demand (`content_rows + TILE_ROWS_READOUT_HEIGHT + TILE_BORDER_ROWS`, at least `MIN_TILE_HEIGHT`). Stacked pieces share a border: a piece at `top` with `framed` rows ends at `top + framed`, the next starts at `top + framed - 1` (`PieceSpan::next_top`).
- Vocabulary, all private to `flow_layout/` with getters: `FlowContent<Id> { Group(Id), Empty(usize) }` (`From` into `TileContent`), `TileRows<Id>` (`new`, `content`, `content_rows`, `framed`), `ContentWindow { first_row }`, `PieceSpan { column, top, framed }`, `SplitPiece` (`at`, `window`, `content_rows: NonZeroU16`), `SplitTile<Id>` (`content`, `head`, `continuations()` over `continued` then `more`, never empty), `FlowPiece<Id> { Whole { tile, at }, Split(SplitTile<Id>) }`, `UnseenTiles { None, Trailing(NonZeroUsize) }`, `FlowLayout<Id> { pieces, unseen }`.
- Splitting outside the last column: a tile that fits whole stays; fewer than `MIN_TILE_HEIGHT` rows left, or a head that would show every content row, moves the tile whole to the next column at top 0; otherwise the head fills the rows left and shows `shown_content_rows(rows)`. Continuations open each following column at top 0 with `first_row` = rows shown so far, ending where their framed rows fit or filling the band and continuing.
- `SplitBuilder<Id, State = SplitPending>` is the only constructor of a `SplitTile`: `start` → `ColumnFit { Whole, Move, Split }`, `continue_into` → `Continue { More, Done(SplitBuilder<Id, SplitReady>) }`, `complete`, `finish`. It owns the row cursor, so windows are contiguous and piece rows sum to the tile's content rows by construction.
- The last column is the last band of at least `MIN_TILE_HEIGHT` rows. There each piece takes `min(demand, rows left - reserve)`, reserving `MIN_TILE_HEIGHT - 1` rows for each following tile the band can still hold, so several pieces can clip below demand; tiles past that capacity are reported as `Trailing(n)`. Each column's last piece grows to its band foot; no state persists between calls and `TILE_DEMAND_STEP` is not applied.
- `shown_content_rows(framed)` mirrors `draw::content_area(frame_inner(rect))`: `framed - TILE_BORDER_ROWS`, less the readout row when more than one interior row remains.

**Files:**
- `crates/tui_pane/src/tiles/flow_layout/mod.rs` — vocabulary types, `FlowLayout::pack`, `framed_rows`, `shown_content_rows`, `fill_column_foot`, `final_framed`; opens with `#![cfg_attr(not(test), expect(dead_code, …))]`
- `crates/tui_pane/src/tiles/flow_layout/split_builder.rs` — `SplitBuilder` with `SplitPending`/`SplitReady` states, `ColumnFit`, `Continue`, `FlowingTile`
- `crates/tui_pane/src/tiles/flow_layout/tests.rs` — 21 packer tests with `u8` ids, including the natedev and hana cases and a contiguity sweep over content rows 1–150 and bands 3–40
- `crates/tui_pane/src/tiles/mod.rs` — `mod flow_layout;`, no re-export

**Binds later work:** the tui_pane flow entry phase removes the `dead_code` expect and widens what it reads to `pub(super)`; it consumes `UnseenTiles::Trailing(n)` as a red `+n unseen` readout on the last placed piece, and shows a clipped piece's shortfall from `content_rows` against `shown_content_rows(framed)`.

**Gotchas:**
- Bands shorter than `MIN_TILE_HEIGHT` are skipped: no piece starts in one, and continuations pass over them.
- A clipped final continuation owns all remaining content rows, so its `content_rows` can exceed the rows it shows.
- The workspace denies `self_named_module_files`: a test module under a self-named file (`draw/tests/…`, `render/tests/…`) needs the `#[path = "…"]` pattern (`crates/cargo-handler/src/terminal.rs:29`).

**Ruled out:** clipping only the last piece of the last column — keeping every tile visible at its floor wins.

### Phase 2 — tui_pane flow entry  · status: todo

**Depends on:** Phase 1

**Blocked by:** tile-fixes — its measured-once grid work (`FrameCellRowAnswers`, `TileCells::prepare_frame`, `TileView::Auto` removed) merges to main and this branch merges main; the showrunner sends word.

#### Work Order

**Goal:** `draw_tile_flow` draws a grid whose tiles flow column to column at their demand: a split tile's head carries `group_title`, its continuation carries `continued_title` and the rows after the head's, and each piece shows its own readout.

**Spec:**
1. Before writing, re-read `tiles/grid.rs`, `tiles/draw.rs`, `tiles/settings.rs`, `pane/frame.rs` and `crates/tui_pane/CHANGELOG.md` as they stand after the merge. The Delegation Context line refs predate tile-fixes; build on its measured-once types and `prepare_frame` rather than the pre-merge shapes. Refs marked "tile-fixes" below are its uncommitted tree and shift with the merge.
2. Public trait in `draw.rs`, beside `TileCells`, re-exported from `tiles/mod.rs` and `lib.rs`:
   ```rust
   /// What an app adds to its [`TileCells`] to have its tiles flow from column to column.
   ///
   /// A tile too tall for the rows left in its column continues at the top of the next.
   /// [`draw_tile_flow`] lays the contents out at their full demand height and shows each
   /// piece its own rows; this names the continuation.
   pub trait FlowTileCells<Id>: TileCells<Id> {
       /// The title of a continuation, given the title its head carries.
       fn continued_title(&self, head: Line<'static>) -> Line<'static>;
   }
   ```
   Planning call, replacing the design's `draw_window` and `continued_title(&self, id) -> Option<Line>`: `draw_tile_flow` draws every split piece itself (item 7) through the scratch-buffer copy `pane/frame.rs` already has, so no consumer reimplements it and `ContentWindow` stays crate-private. `continued_title` takes the head's title, so a continuation carries a title exactly when its head does, with no `Option`. The trait is still the opt-in (ruling 1).
3. Public entry in `draw.rs`, re-exported the same way:
   ```rust
   pub fn draw_tile_flow<Id: Clone + Eq + Debug>(
       buffer: &mut Buffer,
       grid: &mut TileGrid<Id>,
       area: Rect,
       growth: TileGrowth,
       contents: TileGridContents,
       cells: &impl FlowTileCells<Id>,
   )
   ```
   It follows `draw_tile_grid`'s post-merge frame sequence (tile-fixes `draw.rs` ≈268: `set_draw_layout`, `prepare_frame`, `demands`, `add_readout_rows`, `sync_rendered_demands` so slots, cell order and focus stay the grid's), with the divided layout's pieces replaced by flow pieces.
   - Demand source: the packer's input is the `TileDemands` that `cells.demands(widths)` returns this frame, after `add_readout_rows` — the same demand pass `draw_tile_grid` runs. `FrameCellRowAnswers` caches the allocation probes `measure_row_steps` fills (`rows_drawn`/`rows_kept` per allocation), not demands; flow reads nothing from it. `draw_tile_flow` makes the same `demands` calls per frame as `draw_tile_grid` (one per display it measures), however many continuations the layout has.
   - `prepare_frame`'s allocations are flow's: each tile's content area at the width it is measured at (item 5), not `preparation_allocations`' divided rects (tile-fixes `grid.rs` ≈1897), so the widths `prepare_frame` sets equal the widths `demands` is asked at.
   - Frame lifecycle: flow skips `measure_row_steps`' divided row probing (tile-fixes `grid.rs` ≈1285) but keeps the lifecycle work it does: clear `frame_row_usage`, `commit_display()` on a settled frame (≈1715), and set `last_settled_geometry` to `NoCellsFrame` on a flowed frame, so a later divided motion never starts from divided geometry older than the flowed frames. Factor those three out of `measure_row_steps` into one `TileGrid` method both entry points call; `draw_tile_grid`'s order and output are unchanged.
4. Admission (unit director ruling on finding 1): flow admits every group. Under `draw_tile_flow`, `admitted()`'s fit cap (tile-fixes `grid.rs` ≈2132) and `sync_demands`' insert-branch `fits` check (≈2084) do not apply, and `display()` (≈2916) does not fall back to `SummaryAlone` when `holds_in` fails; every live group reaches the packer, and groups the bands cannot show come back as `UnseenTiles::Trailing(n)`. As a type, not a flag: `enum Admission { Fitting, Every }` passed into `sync_demands`; `draw_tile_grid` passes `Fitting`, `draw_tile_flow` passes `Every`, and the public `sync` (≈1994) and `display()` read it from the grid's `DrawnLayout` (item 6: `Divided` → `Fitting`, `Flowed` → `Every`). `TileView::Summary` still draws the summary alone under flow, applied through `commit_display`. `draw_tile_grid` (`Divided`) admission, `holds_in` and summary-only fallback are unchanged.
5. Flow geometry, a `TileGrid` method in `grid.rs` (for example `flow_drawing(area, growth, demands) -> FlowDrawing<Id>`):
   - Column count: `columns(count, growth).len()` with the summary counted once; column rects from `fills` over `area` with shared borders, the same way the grid builds its column rects (`resolved_grid`).
   - The summary keeps its slot at the top of column 0, at its exact framed demand (at least `MIN_TILE_HEIGHT`, at most the column height), over the columns `TileGrowth::widen_summary` lets it widen across, as the grid computes its span today. Each covered column's band starts at the summary's bottom border row; other bands are the whole column.
   - Tiles in cell order become `TileRows` (`FlowContent::Group` for groups, `FlowContent::Empty(n)` for unclaimed cells) at their content rows (item 3's `demands.rows_for(id)` minus the readout row `add_readout_rows` added, or 0 for an empty cell), then `FlowLayout::pack(&bands, tiles)`, which returns a `FlowLayout`.
   - Each tile is measured at the content width of the column its first piece took on the previous flowed frame, or at the narrowest column for a tile not placed yet, matching `content_widths` for the divided grid.
   - Every piece becomes a placement keyed by its cell number; every piece of the focused tile carries the focus. Phase 2 draws the settled flow layout every frame; phase 3 adds motion.
6. The grid records which layout drew it, as a type: `enum DrawnLayout { Divided, Flowed(<each piece's rect and cell number>) }` on `TileGrid`, set by `draw_tile_grid` and `draw_tile_flow`, defaulting to `Divided`. `cell_at` (≈2763) answers against the flowed piece rects when `Flowed`, so a click on any piece of a split focuses that tile. `Divided` behavior is unchanged.
7. Drawing pieces. Factor the per-piece body of `draw_placements` (tile-fixes `draw.rs` ≈356–440) into one helper both entry points call, so piece drawing stays one implementation and `draw_tile_grid` output is unchanged byte for byte. Per flow piece:
   - `Whole` draws through `TileCells::draw` into its content area, as today, so a whole piece cut in an overfull last column shows the consumer's own short view.
   - Split pieces draw their window. Two row counts stay distinct: the tile's demand (`TileRows::content_rows`, every content row the tile has) sizes the scratch, and each piece's owned rows (`SplitPiece::content_rows()`) bound what that piece copies. Render `cells.draw` for the tile once per frame into a scratch `Buffer` one content area wide (the head's content width) and the tile's demand tall, pre-styled the way `draw_at_destination` (`pane/frame.rs` ≈230) pre-styles its scratch (`screen_ground`, then `fill_pane` for the piece's focus); every piece of that tile copies from the same scratch. Each piece copies rows `window.first_row() .. window.first_row() + content_rows()` — its owned window, never past it — into its content area, as many as the area holds and as many columns as its inner width holds; content-area rows past the owned window keep the pre-styled ground (a column's last piece grows to the band foot without owning more rows). Share the pre-style with `pane/frame.rs` through a `pub(crate)` helper there rather than copying it.
   - Readout per piece: `Whole` counts the tile's content rows; each split piece counts its own `content_rows()`. Each is measured at the tile's demand width, so a piece reads green while its rows fit and red once cut (ruling 4). The last column can clip several pieces: every clipped piece shows its own red readout. A clipped final continuation owns every remaining content row (`SplitBuilder::complete`), so its readout reports that full remaining demand against the rows shown: an 11-row continuation in 9 content rows shows window rows 0–8 and reads `11` in red. When `unseen` is `Trailing(n)`, the last placed piece's readout adds `+n unseen` in red.
   - No readout row (unit director ruling on finding 2): when `readout_row` gives `CellTooSmall` for a piece (a three-row piece whose readout gives way to content), its clipped shortfall and its `+n unseen` are written on that piece's bottom border row, red, right-aligned, through `GridLines::add_label` (tile-fixes `pane/frame.rs` ≈628), since a shared border row is drawn by the pass that owns it and a label slides clear of a title already on that row. The text is the readout's narrowest `extent_readout_lines` form that fits, then `+n unseen` when it applies. Every piece is at least `MIN_TILE_HEIGHT`, so a bottom border row always exists. A layout with zero pieces (no band reaches `MIN_TILE_HEIGHT`) has no drawable area and no surface.
   - Titles: the head and a whole piece carry `cells.group_title(id)`; every continuation carries `cells.continued_title(head_title)` when `group_title` is `Some`, else a plain border. Titles still go through `GridLines::add_titled` and stay off while contents are hidden. Empty cells draw their number on their whole piece.
8. Remove phase 1's `#![cfg_attr(not(test), expect(dead_code, …))]` from `flow_layout/mod.rs` and widen to `pub(super)` exactly the items `draw.rs`/`grid.rs` read.
9. `tiles/mod.rs` module doc names `draw_tile_flow` and `FlowTileCells` beside `draw_tile_grid`. `crates/tui_pane/CHANGELOG.md` `## [Unreleased]` `### Added`: `FlowTileCells` and `draw_tile_flow` let an app lay its tiles out as a flow — each tile at its demand, a tile too tall for the rows left in its column continuing at the top of the next under `FlowTileCells::continued_title`, a column's spare rows going to its last tile, and tiles that do not fit counted `+n unseen`; `draw_tile_grid` is unchanged.
10. Tests go in `crates/tui_pane/src/tiles/draw/tests/flow_tests.rs`, declared inside `draw.rs`'s existing `mod tests` block as `#[path = "flow_tests.rs"] mod flow_tests;` (the explicit-path pattern of `crates/cargo-handler/src/terminal.rs` ≈30–32, which keeps clippy's `self_named_module_files` quiet). Inside an inline module of the non-mod-rs file `draw.rs`, a `#[path]` resolves from `tiles/draw/tests/`, so `"flow_tests.rs"` names that file; as a child of `mod tests` it reads that module's fixtures through `super::`. A test `FlowTileCells` whose content row `n` reads `row n` makes window checks textual. Required cases:
    - A continuation's first content row reads the row after the head's last (`row 29` under a 32-row head), a tile spanning three columns reads contiguous rows in all three pieces, and no row appears in two pieces.
    - The continuation's title is the head's title passed through `continued_title`; the head's is `group_title`.
    - A tile after a split starts right below the continuation; a short tile takes only its framed demand and the column's last piece reaches the foot, with no owned row repeated in the rows added below it.
    - Each piece's readout counts its own rows and reads green; several clipped pieces in an overfull last column each show their own red readout; an 11-row final continuation drawn into 9 content rows shows `row` text for the first 9 window rows and reads `11` in red; `Trailing(n)` shows `+n unseen` on the last piece's readout row.
    - No readout row: an area whose one flow band is 7 rows with four tall tiles → three floor pieces and one unseen tile; `+1 unseen` is red and right-aligned on the third piece's bottom border, and each clipped floor piece's shortfall is red on its own bottom border. A layout with zero pieces draws no unseen text and does not panic.
    - Admission: more groups arrive than the divided grid's `fits` admits; under `draw_tile_flow` every one becomes a slot and reaches the packer, and those the bands cannot show are counted in `+n unseen`.
    - Shrinkage: a flowed grid redrawn into an area where `holds_in` fails keeps drawing flow pieces, never the summary alone, with the tiles that no longer fit counted unseen.
    - Divided unchanged: the same arrivals and the same shrinkage through `draw_tile_grid` keep its admission cap and summary-only fallback, byte for byte as before.
    - Demand passes: a counting `FlowTileCells` (a `Cell<usize>` bumped in `demands`) records the same `demands` calls per frame for a tile drawn whole as for the same tile split across three columns, equal to `draw_tile_grid`'s count on the same grid; `prepare_frame`'s allocation widths equal the widths `demands` is asked at.
    - Lifecycle: `set_view(TileView::Summary)` during a grid motion keeps flow pieces until the motion settles, then the settled flowed frame draws the summary alone; a grid drawn by `draw_tile_grid` into an area too small for its cells (a summary-only frame), then by `draw_tile_flow` into the same area with `TileView::Cells`, draws flow pieces.
    - A click on a continuation focuses its tile (`cell_at` under `Flowed`).
    - The summary keeps the top of column 0 and its title.
    - `draw_tile_grid` output for the existing fixtures is unchanged (the existing draw.rs tests stay green unedited).

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — `FlowTileCells`, `draw_tile_flow`, shared per-piece helper, window drawing, border readout labels, `#[path = "flow_tests.rs"] mod flow_tests;` inside `mod tests`
- `crates/tui_pane/src/tiles/grid.rs` — flow geometry, `Admission`, `DrawnLayout`, frame lifecycle method factored out of `measure_row_steps`, `cell_at`, `display`
- `crates/tui_pane/src/tiles/flow_layout/mod.rs` — drop the dead-code expectation, widen read items
- `crates/tui_pane/src/pane/frame.rs` — `pub(crate)` scratch pre-style helper shared with window drawing
- `crates/tui_pane/src/tiles/mod.rs` — re-exports, module doc
- `crates/tui_pane/src/lib.rs` — re-exports `FlowTileCells`, `draw_tile_flow`
- `crates/tui_pane/CHANGELOG.md` — `### Added` entry
- `crates/tui_pane/src/tiles/draw/tests/flow_tests.rs` — new: flow drawing tests

**Workers:** 1 writer + 1 tester — `grid.rs` and `draw.rs` change together and cannot split; the tests are their own file.
- `impl` — `crates/tui_pane/src/tiles/draw.rs` (including the `#[path]` test declaration), `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/flow_layout/mod.rs`, `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/CHANGELOG.md`; hub: `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` (re-exports)
- `test` — `crates/tui_pane/src/tiles/draw/tests/flow_tests.rs`: every case in Spec item 10, written against the signatures in items 2–3

**Constraints from prior phases:**
- Phase 1 built `crates/tui_pane/src/tiles/flow_layout/` (`mod.rs` + `split_builder.rs`, declared `mod flow_layout;` in `tiles/mod.rs`, not re-exported): `FlowContent<Id> { Group(Id), Empty(usize) }` with `From` into `TileContent<Id>`; `TileRows<Id>::new(content, content_rows)` and `framed()` (content + `TILE_ROWS_READOUT_HEIGHT` + `TILE_BORDER_ROWS`, at least `MIN_TILE_HEIGHT`); `ContentWindow { first_row }`; `PieceSpan { column, top, framed }`; `SplitPiece { at, window, content_rows: NonZeroU16 }`; `SplitTile<Id>` (head plus a non-empty run of continuations, contiguous windows, built only by the packer's `SplitBuilder`; `head()`, `continuations()`); `FlowPiece<Id> { Whole { tile, at }, Split(SplitTile) }`; `UnseenTiles { None, Trailing(NonZeroUsize) }`; `FlowLayout::pack(bands: &[u16], tiles: Vec<TileRows<Id>>) -> FlowLayout<Id> { pieces, unseen }`; `shown_content_rows(framed)`. Items are private behind `#![cfg_attr(not(test), expect(dead_code, …))]`, which this phase removes.
- Pack units: framed rows on shared borders; a band is a column's rows below the summary; a piece at `top` ends at `top + framed` and the next starts at `top + framed - 1`; each column's last piece reaches the band foot (`fill_column_foot` grows its `framed`, never its `content_rows`); a tile continues across as many columns as it needs.
- Last column (`final_framed`): each piece takes `min(demand, rows left - reserve)` with the reserve keeping every later tile at `MIN_TILE_HEIGHT`, so several pieces can clip and every clipped piece shows its own red readout. A clipped final continuation owns every remaining content row (`SplitBuilder::complete`), so its readout reports that full remaining demand against the rows shown. Tiles that do not fit even at the floor are `Trailing(n)`; a layout whose bands all sit under `MIN_TILE_HEIGHT` has zero pieces.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane` green with every Spec item 10 case, the admission and shrinkage cases included, and the existing `draw.rs` tests green unedited (`draw_tile_grid` output byte-identical); `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` green with no snapshot edited; `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` green (it still calls `draw_tile_grid`); `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` clean (no `self_named_module_files` finding); the CHANGELOG entry is present.

### Phase 3 — tui_pane flow motion  · status: todo

**Depends on:** Phase 2

**Blocked by:** tile-fixes — the same merge as phase 2.

#### Work Order

**Goal:** flow pieces move with the grid's existing motion: a piece that moves or resizes slides, and a continuation opens and closes the way a tile does, with no doubled or torn rows at any point of the clock.

**Spec:**
1. Re-read `tiles/grid.rs` motion after the merge (`Transition`, `Step`, `GridMotion`, `progress`, `tick`, `eased`, `column_bands`, `moving_cell`, `wrapping_cell`, `BandPieceMotion`, `place_band_pieces`, `TileSettings::animation_millis`) and `tiles/draw.rs` (`draw_tile_flow` from phase 2). Jumping is not acceptable, because cargo-handler's tiles already slide (ruling 3).
2. Flow pieces are keyed by the tile's cell content and a part: `enum FlowPart { First, Continuation(NonZeroUsize) }`, where `First` is a `Whole` piece or a split's head and `Continuation(k)` is its k-th continuation. A tile becoming split keeps its `First` piece and gains continuations; a split shrinking or rejoining keeps `First` and loses its highest continuations.
3. `TileGrid` keeps the flow geometry its last frame drew. When a newly packed geometry differs (any piece's column, top or framed rows, a continuation forming or leaving, a tile joining or leaving), a flow transition starts on the grid's clock, timed by `TileSettings::animation_millis`, from the geometry drawn at that frame to the new one. A change arriving mid-motion — a demand change or a resize — starts the next transition from the geometry drawn at that instant, so motion never jumps. A focus change alone moves nothing, since pack ignores focus.
4. Repaint contract: `tick()` (tile-fixes `grid.rs` ≈1956) is the one public poll cargo-handler uses for animation frames (`terminal.rs` ≈154). It returns `true` while a flow transition is in flight, including a flow-only transition while `GridMotion` is `Settled`, completes the flow transition once its progress reaches `PROGRESS_SCALE`, and returns `false` once both the grid motion and the flow transition have settled. Phase 4 adds no animation driver.
5. Motion per piece, through the existing band machinery so dividers stay one line per boundary: present before and after in the same column → `BandPieceMotion::Resident` (slides and resizes); `First` changing column → `wrapping_cell`; a `Continuation` appearing → `Entering`, as a tile opens; a `Continuation` leaving → `Leaving`, or `ClosingWithColumn` when its column closes, as a tile closes; tiles joining or leaving use the grid's opening and closing motion. Feed both flow geometries to `place_band_pieces`. If it needs a `Grid` for before and after, build them from the flow columns and bands, or factor its band topology out so both callers share it; never write a second divider placement.
6. A split piece in motion still draws its window (phase 2 item 7), clipped by its moving frame, and copies only its owned window `window.first_row() .. window.first_row() + content_rows()`: a frame that enlarges mid-motion shows the pre-styled ground below its owned rows, never rows another piece of the tile owns.
7. Tests go in `crates/tui_pane/src/tiles/draw/tests/flow_motion_tests.rs`, declared inside `draw.rs`'s `mod tests` block as `#[path = "flow_motion_tests.rs"] mod flow_motion_tests;` (resolves from `tiles/draw/tests/` as phase 2's `flow_tests` does; reads `mod tests`' fixtures through `super::`), drawing through `drawing_at` (or its post-merge equivalent) at raw progress 0, ¼, ½, ¾ and settled. Required cases:
   - A split forming (a tile's demand grows past its column's rows): at every sample each content row of that tile appears at most once across its pieces, and no column shows two stacked border rows.
   - A split leaving (the demand shrinks back): the same two checks, and the settled frame has no continuation.
   - A tile joining before a split tile slides the later pieces; no sample jumps a piece by more than its eased step.
   - Retarget: a second demand change at raw ½ starts a transition whose first frame draws every piece where the ½ frame drew it, and every later sample keeps each content row in at most one piece; the same for a resize at raw ½.
   - Owned window: a continuation whose frame enlarges mid-motion shows no row its sibling piece owns at any sample.
   - Repaint: after a flow-only transition starts, `tick()` returns `true` until it settles and `false` after, settled through the grid's test hook (`finish_motion_for_test`/`settle_for_test` or its flow equivalent), never a sleep.
   - A focus change starts no flow transition.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — flow geometry memory, flow transition, `tick` over flow motion, band pieces for flow
- `crates/tui_pane/src/tiles/draw.rs` — `draw_tile_flow` draws the moving geometry; `#[path = "flow_motion_tests.rs"] mod flow_motion_tests;` inside `mod tests`
- `crates/tui_pane/src/tiles/flow_layout/mod.rs` — `FlowPart`, if the key lives with the packer
- `crates/tui_pane/src/tiles/draw/tests/flow_motion_tests.rs` — new: motion tests

**Workers:** 1 writer + 1 tester — the motion is one change across `grid.rs` and `draw.rs`; the tests are their own file.
- `impl` — `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs` (including the `#[path]` test declaration), `crates/tui_pane/src/tiles/flow_layout/mod.rs`
- `test` — `crates/tui_pane/src/tiles/draw/tests/flow_motion_tests.rs`: every case in Spec item 7, against `draw_tile_flow`, `drawing_at` and `tick`

**Constraints from prior phases:**
- Phase 1's packer (`flow_layout/`): `FlowLayout::pack(bands, tiles) -> FlowLayout<Id> { pieces, unseen }` with `FlowPiece::{Whole { tile, at }, Split(SplitTile)}`, `SplitTile` a head plus a non-empty run of contiguous continuations (`SplitPiece { at, window, content_rows }`); pure and stateless per call.
- Phase 2's entry: public `FlowTileCells<Id>: TileCells<Id>` with `continued_title(&self, head: Line<'static>) -> Line<'static>` and `draw_tile_flow(buffer, grid, area, growth, contents, cells)` in `draw.rs`; flow geometry on `TileGrid` in `grid.rs`; `DrawnLayout::{Divided, Flowed(..)}` drives `cell_at`, `display()` and `Admission` (flow admits every group; unseen groups are `Trailing(n)`, never a summary-only fallback); demand comes from the `demands` pass, no extra pass per continuation; flow keeps `measure_row_steps`' lifecycle work through a shared method. Split pieces draw from one scratch per tile at its full demand height, pre-styled by a shared `pane/frame.rs` helper, each copying only its owned window. Readouts: each piece its own; every clipped piece red; `+n unseen` on the last piece's readout row, or on its bottom border when it has none. The per-piece drawing helper is shared with `draw_tile_grid`, whose output is unchanged. Phase 2 drew the settled flow layout each frame, with no motion.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test tui_pane` green with every Spec item 7 case, the retarget and `tick()` cases included; `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` green with no snapshot edited; `bash ~/.claude/scripts/direct/verify.sh lint tui_pane` clean.

### Phase 4 — cargo-handler on flow  · status: todo

**Depends on:** Phase 3

#### Work Order

**Goal:** cargo-handler draws its grid through `draw_tile_flow`, so a tile too tall for its column continues in the next under `<name> · <role> · continued`, and short tiles take only their rows.

**Spec:**
1. `crates/cargo-handler/src/constants.rs`: `pub(crate) const CONTINUED_TITLE_LABEL: &str = "continued";` beside `HEADING_SEPARATOR` (≈182), with a doc comment (what a continuation's title adds after its head's).
2. `crates/cargo-handler/src/render.rs`: `impl FlowTileCells<AgentCell> for Cells<'_>` after the `TileCells` impl (≈141–232). `continued_title(&self, head)` returns `head` with one span appended: `format!("{HEADING_SEPARATOR}{CONTINUED_TITLE_LABEL}")` in `Style::default().fg(label_color())`, so `hana · showrunner` continues as `hana · showrunner · continued` with the name's hue and the role's label color kept.
3. `draw_panes` (≈87) calls `tui_pane::draw_tile_flow` with the same arguments in place of `tui_pane::draw_tile_grid` (≈95); its doc comment names `draw_tile_flow`. The test helper `drawn_buffer_in` (≈791) switches the same way, so tests draw what the app draws.
4. `agent_cell.rs` is not edited: `draw_tile_flow` lays each split piece out at its full demand height, so `CellView::fitted` keeps the full view; the compressed view shows only in a whole piece cut in an overfull last column.
5. Existing `render.rs` goldens that pin divided-grid geometry may change, since short tiles no longer take an even share and every agent now gets a tile. Update each such expectation to the flow layout and keep every assertion about content, titles, hues and role labels.
6. `crates/cargo-handler/CHANGELOG.md` `## [Unreleased]`: a tile with more rows than its column has continues at the top of the next column, titled `· continued`; tiles after it move along and short tiles take only the rows they need.
7. Tests go in `crates/cargo-handler/src/render/tests/flow_tests.rs`, declared inside `render.rs`'s `mod tests` block as `#[path = "flow_tests.rs"] mod flow_tests;` (the `terminal.rs` ≈30–32 explicit-path pattern; inside an inline module of the non-mod-rs file `render.rs` the path resolves from `src/render/tests/`), reusing `boss`, `drawn_buffer_in` and `drawn_grid` through `super::`. Required cases:
   - A two-column body with one agent whose cell needs more rows than its column: the next column's top title reads `<name> · <role> · continued`, and its first content row is the row after the head's last, compared against the same cell drawn at full height into a tall buffer.
   - A short agent's tile takes only its demand, with the column's last tile reaching the foot.
   - The continuation's name span keeps the agent's hue, and ` · continued` is in the label color.

**Files:**
- `crates/cargo-handler/src/constants.rs` — `CONTINUED_TITLE_LABEL`
- `crates/cargo-handler/src/render.rs` — `FlowTileCells` impl, `draw_tile_flow` in `draw_panes` and `drawn_buffer_in`, goldens updated, `#[path = "flow_tests.rs"] mod flow_tests;` inside `mod tests`
- `crates/cargo-handler/CHANGELOG.md` — `## [Unreleased]` entry
- `crates/cargo-handler/src/render/tests/flow_tests.rs` — new: flow tests

**Workers:** 1 writer + 1 tester — the impl, call switch and golden updates sit in `render.rs`; the new tests are their own file.
- `impl` — `crates/cargo-handler/src/render.rs` (including the `#[path]` test declaration), `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`
- `test` — `crates/cargo-handler/src/render/tests/flow_tests.rs`: every case in Spec item 7

**Constraints from prior phases:**
- Phase 2: `tui_pane::FlowTileCells<Id>: TileCells<Id>` has one required method, `fn continued_title(&self, head: Line<'static>) -> Line<'static>`, called only when `group_title` returned `Some`; `tui_pane::draw_tile_flow(buffer, grid, area, growth, contents, cells)` takes the same arguments as `draw_tile_grid`. tui_pane draws split pieces itself (full-height scratch layout, each piece copying its owned rows), so the consumer implements no window drawing. Each piece shows its own readout. A tile may continue across several columns, and every continuation carries the continued title. Flow admits every group: agents the grid cannot show are counted `+n unseen` in red (on the last piece's readout row, or its bottom border for a three-row piece), and a small terminal never switches to the summary alone; `TileView::Summary` from the config still does.
- Phase 3: flow pieces slide, and continuations open and close like tiles, on the grid's clock; `TileGrid::tick()` keeps returning `true` through flow motion, so the existing `terminal.rs` ≈154 poll drives it and nothing in cargo-handler drives that motion.
- `cell_at` answers against flowed pieces once `draw_tile_flow` has drawn, so clicks keep working unchanged in `app.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/direct/verify.sh test cargo-handler` green with every Spec item 7 case; `bash ~/.claude/scripts/direct/verify.sh lint cargo-handler` clean (no `self_named_module_files` finding); `bash ~/.claude/scripts/direct/verify.sh test cargo-tile` green.

### Phase 5 — Live check  · status: todo

**Depends on:** Phase 4

#### Work Order

**Goal:** the real natedev grid at the user's terminal size shows every row of natedev and hana, `· continued` titles on continuations, compact short tiles, and a split that slides as it forms and leaves.

**Spec:**
1. The unit director installs the checkpoint binary and runs cargo-handler on natedev at the user's terminal size, with the Mac shown.
2. Check: the natedev and hana tiles show every content row across their pieces, each continuation's title reads `<name> · <role> · continued`, short tiles hold only their rows with no gap at a column's foot, and every readout reads green unless the grid is overfull.
3. Shrink the terminal until the grid is overfull and confirm the readouts: every clipped piece's readout is red, `+n unseen` appears in red when tiles do not fit (on the last piece's readout row, or on its bottom border when that piece is three rows), and the grid keeps flowing rather than switching to the summary alone.
4. Make a split form and leave (a tile's demand crossing its column's rows, or a terminal resize) and confirm the pieces slide with no doubled or torn rows.
5. Send the shots to the showrunner, and record the result here.

**Files:**
- `docs/cargo-port/remove-running-section-handler-overflow.md`

**Workers:** none — the unit director runs the live check.

**Acceptance gate:** shots of the live grid show every check in Spec items 2 and 3, the red `+n unseen` readout included; a split forming and leaving slides cleanly; and the showrunner has the shots.
