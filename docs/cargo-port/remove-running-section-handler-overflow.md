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
  - `crates/cargo-handler/src/render.rs` — `draw_panes` ≈86 calls `tui_pane::draw_tile_grid` ≈94 (the design doc named `terminal.rs`; the call is here); `Cells` ≈106; `impl TileCells<AgentCell> for Cells` ≈136; `group_title` ≈196 (name in `Role::Rainbow(entry.hue)`, then `entry.role_label.suffix()` in `label_color()`); `mod tests` ≈247 with fixtures `boss` ≈700, `drawn_buffer` ≈759, `drawn_buffer_in` ≈765 (also calls `draw_tile_grid` ≈773), `drawn_grid` ≈785.
  - `crates/cargo-handler/src/agent_cell.rs` — `CellView` ≈354, `CellView::fitted` ≈377 (compressed when the area is shorter than the full view), `height` ≈421, `draw` ≈468, `draw_view` ≈489.
  - `crates/cargo-handler/src/census/placement.rs` — `RoleLabel::suffix` ≈77 (`" · <role>"` via `HEADING_SEPARATOR`).
  - `crates/cargo-handler/src/constants.rs` — `HEADING_SEPARATOR` = `" · "` ≈203.
  - `crates/cargo-tile/src/render.rs` — `draw_tile_grid` call ≈302, `impl TileCells<InvocationId> for Cells` ≈384; read only, it never calls the flow entry point.
- **Test lanes:** `tui_pane` has `crates/tui_pane/tests/`, but the flow code is crate-private, so its tests are unit-test files under `src/` declared with `#[cfg(test)] mod …;`; `cargo-handler`: none (unit-test files under `src/`, the crate's pattern in `terminal.rs` ≈30 and `census/scan.rs` ≈22). A test file declared inside an existing `mod tests` block is a child of it and reads that block's private fixtures through `super::`.
- **Build:** `bash ~/.claude/scripts/delegate/verify.sh check tui_pane`; `bash ~/.claude/scripts/delegate/verify.sh check cargo-handler`
- **Test:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane`; `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler`; regression for the unchanged consumer: `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile`
- **Lint:** `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane`; `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler`
- **Invariants:**
  - Nothing on the UI thread runs a command or reads a file.
  - Tests run with nextest; format only with `cargo +nightly fmt`; no test takes a second.
  - Invalid states are unrepresentable: use types (checked constructors, enums, type state), never procedural guards (user rule 2026-10-15). No bare `Option` in new domain types.
  - Flow layout is opt-in by entry point and trait, never a flag (showrunner ruling 1): a consumer gets it only by calling `draw_tile_flow` with `cells: &impl FlowTileCells<Id>`. There is no boolean on `TileGrowth` or `TileGrid`.
  - cargo-tile behavior and snapshots stay unchanged: `draw_tile_grid` keeps its output byte for byte, and cargo-tile never calls the flow entry point.
  - Each frame measures each tile once: flow reads demand from the same per-frame measurement `draw_tile_grid` uses (tile-fixes' `FrameCellRowAnswers`), never a second pass of its own.
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

#### Work Order

**Goal:** a pure `tiles/flow_layout/` module packs tiles top to bottom, column by column, each at its measured demand; a tile too tall for the rows left in its column continues at the top of the next column, across as many columns as it needs, with contiguous rows.

**Spec:**
1. New module `crates/tui_pane/src/tiles/flow_layout/` (as built: `mod.rs` holds the vocabulary types and the `FlowLayout` anchor; the split builder cluster is the leaf `split_builder.rs`), declared as `mod flow_layout;` in `crates/tui_pane/src/tiles/mod.rs` after `mod draw;` (≈19). No re-export, and no edit to `grid.rs` or `draw.rs` (production coordination: tile-fixes owns those files until its work merges). Module doc: tiles packed top to bottom, column by column, each at its demand; a tile too tall for the rows left in its column continues at the top of the next.
2. Nothing outside `flow_layout/` reads it until phase 2, so its items stay private (cargo-mend narrows unread wider visibility) and the file opens with `#![cfg_attr(not(test), expect(dead_code, reason = "draw_tile_flow reads the packer from the flow entry phase on"))]`. Phase 2 removes that attribute and widens what it reads to `pub(super)`.
3. Units. Every row count is framed rows unless named content rows. A tile's framed rows are its content rows plus `TILE_ROWS_READOUT_HEIGHT` plus `TILE_BORDER_ROWS`, never below `MIN_TILE_HEIGHT`. Each column has a band: the rows its flowed pieces divide (phase 2 passes the column's height below the summary). Stacked pieces share a border, as `disjoint_demands` (`grid.rs` ≈3663) counts them: a piece at `top` with `framed` rows ends at `top + framed`, and the next piece starts at `top + framed - 1`. A piece fits when `top + framed <= band`.
4. Types (doc comment on every item; derive `Clone, Debug, Eq, PartialEq` where they hold data):
   ```rust
   /// What a flowed tile draws. The summary keeps its own slot and never flows.
   enum FlowContent<Id> { Group(Id), Empty(usize) }
   impl<Id> From<FlowContent<Id>> for TileContent<Id>   // Group -> Group, Empty -> Empty

   /// One tile and the content rows its contents ask for, readout and borders excluded.
   struct TileRows<Id> { content: FlowContent<Id>, content_rows: u16 }
   impl<Id> TileRows<Id> {
       const fn new(content: FlowContent<Id>, content_rows: u16) -> Self;
       fn content(&self) -> &FlowContent<Id>;
       const fn content_rows(&self) -> u16;
       /// content_rows + TILE_ROWS_READOUT_HEIGHT + TILE_BORDER_ROWS, at least MIN_TILE_HEIGHT.
       fn framed(&self) -> u16;
   }

   /// Rows of a split tile's contents one piece shows: the contents laid out at their
   /// full demand height, read from `first_row` down.
   #[derive(Clone, Copy, Debug, Eq, PartialEq)]
   struct ContentWindow { first_row: u16 }   // read through `const fn first_row(self) -> u16`

   /// Where a piece sits in its column's band.
   struct PieceSpan { column: usize, top: u16, framed: u16 }   // read through getters

   /// One piece of a split tile: where it sits, the window it shows, and the content rows
   /// it owns (at least one).
   struct SplitPiece { at: PieceSpan, window: ContentWindow, content_rows: NonZeroU16 }

   /// A tile shown in a head and one or more continuations. Built only by `SplitBuilder`,
   /// so each piece's `window.first_row` equals the previous piece's `first_row + content_rows`,
   /// each continuation opens the next column at top 0, and the pieces' rows sum to the
   /// tile's content rows: no gap, no overlap.
   struct SplitTile<Id> { content: FlowContent<Id>, head: SplitPiece, continued: SplitPiece, more: Vec<SplitPiece> }

   /// The packer's only way to make a SplitTile: it owns the row cursor.
   struct SplitBuilder<Id> { … }   // private to pack; start(tile, head_at) -> ColumnFit, continue_into(column, band) -> Continue, finish() -> SplitTile

   /// One tile as the flow places it, in tile order.
   enum FlowPiece<Id> {
       /// Drawn whole in one column.
       Whole { tile: TileRows<Id>, at: PieceSpan },
       /// A head at the foot of one column and continuations at the top of the next ones.
       Split(SplitTile<Id>),
   }

   /// Tiles the grid could not show even at MIN_TILE_HEIGHT, all trailing in cell order.
   enum UnseenTiles { None, Trailing(NonZeroUsize) }

   /// The packed grid.
   struct FlowLayout<Id> { pieces: Vec<FlowPiece<Id>>, unseen: UnseenTiles }

   fn pack<Id>(bands: &[u16], tiles: Vec<TileRows<Id>>) -> FlowLayout<Id>;
   ```
   `SplitTile` reads: `content()`, `head()`, and `continuations()` (an iterator over `continued` then `more`, never empty); every continuation draws the continued title. `SplitPiece` reads `at()`, `window()`, `content_rows()`. Fields are private to `flow_layout/`; `PieceSpan`, `ContentWindow`, `SplitPiece` and `SplitTile` are built only by the packer, never assembled by callers.
5. `fn shown_content_rows(framed: u16) -> u16` — content rows a piece of `framed` rows shows, mirroring `draw::content_area(frame_inner(rect))` for any width above `TILE_ROWS_RIGHT_INSET`: `inner = framed - TILE_BORDER_ROWS`; `inner - TILE_ROWS_READOUT_HEIGHT` when `inner > TILE_ROWS_READOUT_HEIGHT`, else `inner` (the readout gives way in a one-row cell, as `readout_row` ≈764 does).
6. Splitting, inside `SplitBuilder`. A tile at `top` in a column that is not the last, with `rows = band - top` left: `framed() <= rows` → it fits whole. `rows < MIN_TILE_HEIGHT` → it moves on whole to the next column at top 0. Otherwise the head takes `framed = rows` and shows `shown_content_rows(rows)` content rows; the rest is built through `NonZeroU16::new`, and zero (the head alone would show everything with its readout cut) also moves on whole. Each continuation opens the next column at top 0 with `window.first_row` = rows shown so far: when its framed rows (rest + readout + borders) fit the band it ends there and the cursor sits at `framed - 1`; when they do not and that column is not the last, it fills the band (`shown_content_rows(band)` rows) and the tile continues into the next column. A tile continues across as many columns as it needs.
7. `FlowLayout::pack(bands, tiles)`: the column count is `bands.len()`; tiles go in order with a cursor (`column`, `top`), per item 6 in every column but the last.
   - The last column: no further continuation, since there is no next column. Every tile still to place, and the continuation reaching it, keeps at least `MIN_TILE_HEIGHT`: each takes `min(framed, rows left - reserve)` with `reserve = (MIN_TILE_HEIGHT - 1) × tiles after it`. A piece is clipped below its demand only when the floors after it need its rows, and every clipped piece's readout shows the shortfall in red (ruling 4); with several large tiles in the last column more than one piece can clip, since keeping every tile visible at its floor wins (unit director ruling at phase 1 review). Only when even every remaining tile at `MIN_TILE_HEIGHT` cannot fit does the column stop at the last tile whose floor fits; that piece is clipped, the tiles after it are unseen, and `unseen = Trailing(n)` names how many, so phase 2's readout on that last piece says `+n unseen` in red. No tile is ever hidden without the readout saying so.
   - Leftover: each column's last piece grows to the band's foot (`framed = band - top`), so no gap is drawn. A piece that fills its band already ends there.
   - Nothing is remembered between calls: short tiles hold only their demand on every call, and rows return each frame. There is no stickiness and no demand step (`TILE_DEMAND_STEP` is not applied).
8. Tests go in `crates/tui_pane/src/tiles/flow_layout/tests.rs`, declared in `flow_layout/mod.rs` as `#[cfg(test)] mod tests;`, using `u8` ids. Required cases:
   - natedev: a first tile of 40 content rows (framed 43) in a 32-row first band splits: head (0, 0, 32) shows content rows 0–28, one continuation opens column 1 at top 0 with `first_row` 29, 11 content rows, framed 14; the next tile starts at column 1 top 13.
   - hana: a tile taller than two columns spans three pieces (head, two continuations) in three 40-row bands, with each piece's `first_row` equal to the previous piece's end and the rows summing to the tile's content rows.
   - Short tiles give rows back: three tiles of 2 content rows in one 40-row band sit at framed 5 each, tops 0, 4 and 8, and the last grows to the band foot (framed 32).
   - A split that would leave fewer than `MIN_TILE_HEIGHT` rows moves the tile whole: with 2 rows left the tile opens the next column at top 0, and the previous piece grows to the foot.
   - A head that would show every content row (rest of zero) moves on instead of splitting.
   - An overfull grid keeps every tile at `MIN_TILE_HEIGHT` in the last column and clips only its last piece below demand; `unseen` is `None`.
   - More tiles than the last column holds at the floor: the last placed piece is clipped and `unseen` is `Trailing(n)` with n the count left out.
   - Contiguity sweep: over content rows 1–150 and bands of 3–40 rows, every split tile's pieces have contiguous windows and their rows sum to the tile's content rows.
   - `shown_content_rows` agrees with `draw::content_area(frame_inner(Rect::new(0, 0, 40, framed))).height` for framed 3–12.
   - A single band packs every fitting tile whole with no split.

**Files:**
- `crates/tui_pane/src/tiles/flow_layout/mod.rs` — new: vocabulary types, `FlowLayout::pack`, `shown_content_rows`
- `crates/tui_pane/src/tiles/flow_layout/split_builder.rs` — new: `SplitBuilder` and its states
- `crates/tui_pane/src/tiles/mod.rs` — `mod flow_layout;`
- `crates/tui_pane/src/tiles/flow_layout/tests.rs` — new: packer unit tests

**Workers:** 1 writer + 1 tester — the packer and its tests are separate files.
- `impl` — `crates/tui_pane/src/tiles/flow_layout/mod.rs`, `crates/tui_pane/src/tiles/flow_layout/split_builder.rs`; hub: `crates/tui_pane/src/tiles/mod.rs` (the `mod flow_layout;` line)
- `test` — `crates/tui_pane/src/tiles/flow_layout/tests.rs`: every case in Spec item 8, written against the signatures in Spec item 4

**Constraints from prior phases:** none.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` green with every Spec item 8 case; `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` clean; `git diff` touches neither `tiles/grid.rs` nor `tiles/draw.rs`.

### Phase 2 — tui_pane flow entry  · status: todo

**Depends on:** Phase 1

**Blocked by:** tile-fixes — its measured-once grid work (`FrameCellRowAnswers`, `TileCells::prepare_frame`, `TileView::Auto` removed) merges to main and this branch merges main; the showrunner sends word.

#### Work Order

**Goal:** `draw_tile_flow` draws a grid whose tiles flow column to column at their demand: a split tile's head carries `group_title`, its continuation carries `continued_title` and the rows after the head's, and each piece shows its own readout.

**Spec:**
1. Before writing, re-read `tiles/grid.rs`, `tiles/draw.rs`, `tiles/settings.rs` and `crates/tui_pane/CHANGELOG.md` as they stand after the merge. The Delegation Context line refs predate tile-fixes; build on its measured-once types and `prepare_frame` rather than the pre-merge shapes.
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
   Planning call, replacing the design's `draw_window` and `continued_title(&self, id) -> Option<Line>`: `draw_tile_flow` draws every split piece itself (item 6) through the scratch-buffer copy `pane/frame.rs` already has, so no consumer reimplements it and `ContentWindow` stays crate-private. `continued_title` takes the head's title, so a continuation carries a title exactly when its head does, with no `Option`. The trait is still the opt-in (ruling 1).
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
   It follows `draw_tile_grid`'s post-merge frame sequence (layout recorded, `prepare_frame`, `demands`, readout rows added, `sync_rendered_demands` so slots, cell order and focus stay the grid's), with no more `demands` calls than `draw_tile_grid` makes, and with the divided layout's pieces replaced by flow pieces. The divided layout's per-column row probing (`measure_row_steps`) is not needed for flow pieces. `display()` and `holds_in` decide summary-alone exactly as for the divided grid.
4. Flow geometry, a `TileGrid` method in `grid.rs` (for example `flow_drawing(area, growth, demands) -> FlowDrawing<Id>`):
   - Column count: `columns(count, growth).len()` with the summary counted once; column rects from `fills` over `area` with shared borders, the same way the grid builds its column rects (`resolved_grid`).
   - The summary keeps its slot at the top of column 0, at its exact framed demand (at least `MIN_TILE_HEIGHT`, at most the column height), over the columns `TileGrowth::widen_summary` lets it widen across, as the grid computes its span today. Each covered column's band starts at the summary's bottom border row; other bands are the whole column.
   - Tiles in cell order become `TileRows` (`FlowContent::Group` for groups, `FlowContent::Empty(n)` for unclaimed cells) at their content rows (`demands.rows_for(id)` minus the readout row `add_readout_rows` added, or 0 for an empty cell), then `FlowLayout::pack(&bands, tiles)`, which returns a `FlowLayout`.
   - Each tile is measured at the content width of the column its first piece took on the previous flowed frame, or at the narrowest column for a tile not placed yet, matching `content_widths` for the divided grid.
   - Every piece becomes a placement keyed by its cell number; both pieces of the focused tile carry the focus. Phase 2 draws the settled flow layout every frame; phase 3 adds motion.
5. The grid records which layout drew it, as a type: `enum DrawnLayout { Divided, Flowed(<each piece's rect and cell number>) }` on `TileGrid`, set by `draw_tile_grid` and `draw_tile_flow`, defaulting to `Divided`. `cell_at` (≈2763) answers against the flowed piece rects when `Flowed`, so a click on either piece of a split focuses that tile. `Divided` behavior is unchanged.
6. Drawing pieces. Factor the per-piece body of `draw_placements` (≈343–433) into one helper both entry points call, so piece drawing stays one implementation and `draw_tile_grid` output is unchanged byte for byte. Per flow piece:
   - `Whole` draws through `TileCells::draw` into its content area, as today, so a whole piece cut in an overfull last column shows the consumer's own short view.
   - Split pieces draw their window: render `cells.draw` for the tile into a scratch `Buffer` one content area wide (the head's content width) and `content_rows` tall, pre-styled the way `draw_at_destination` (`pane/frame.rs` ≈230) pre-styles its scratch (`screen_ground`, then `fill_pane` for the piece's focus), then copy rows `window.first_row()..` into the piece's content area, as many as it holds and as many columns as its inner width holds. Share the pre-style with `pane/frame.rs` through a `pub(crate)` helper there rather than copying it.
   - Readout per piece: `Whole` counts the tile's content rows; each split piece counts its own `content_rows()`. When `unseen` is `Trailing(n)`, the last placed piece's readout adds `+n unseen` in red. Each is measured at the tile's demand width, so a piece reads green while its rows fit and red once cut (the overfull last piece, ruling 4).
   - Titles: the head and a whole piece carry `cells.group_title(id)`; every continuation carries `cells.continued_title(head_title)` when `group_title` is `Some`, else a plain border. Titles still go through `GridLines::add_titled` and stay off while contents are hidden. Empty cells draw their number on their whole piece.
7. Remove phase 1's `#![cfg_attr(not(test), expect(dead_code, …))]` from `flow_layout/mod.rs` and widen to `pub(super)` exactly the items `draw.rs`/`grid.rs` read.
8. `tiles/mod.rs` module doc names `draw_tile_flow` and `FlowTileCells` beside `draw_tile_grid`. `crates/tui_pane/CHANGELOG.md` `## [Unreleased]` `### Added`: `FlowTileCells` and `draw_tile_flow` let an app lay its tiles out as a flow — each tile at its demand, a tile too tall for the rows left in its column continuing at the top of the next under `FlowTileCells::continued_title`, and a column's spare rows going to its last tile; `draw_tile_grid` is unchanged.
9. Tests go in `crates/tui_pane/src/tiles/draw/tests/flow_tests.rs`, declared inside `draw.rs`'s existing `mod tests` as `mod flow_tests;`, so they reuse that module's fixtures through `super::`. A test `FlowTileCells` whose content row `n` reads `row n` makes window checks textual. Required cases:
   - A continuation's first content row reads the row after the head's last (`row 29` under a 32-row head), a tile spanning three columns reads contiguous rows in all three pieces, and no row appears in two pieces.
   - The continuation's title is the head's title passed through `continued_title`; the head's is `group_title`.
   - A tile after a split starts right below the continuation; a short tile takes only its framed demand and the column's last piece reaches the foot.
   - Each piece's readout counts its own rows and reads green; an overfull last piece reads red; `Trailing(n)` shows `+n unseen`.
   - A click on the continuation focuses its tile (`cell_at` under `Flowed`).
   - The summary keeps the top of column 0 and its title.
   - `draw_tile_grid` output for the existing fixtures is unchanged (the existing draw.rs snapshot tests stay green unedited).

**Files:**
- `crates/tui_pane/src/tiles/draw.rs` — `FlowTileCells`, `draw_tile_flow`, shared per-piece helper, window drawing, `mod flow_tests;` inside `mod tests`
- `crates/tui_pane/src/tiles/grid.rs` — flow geometry, `DrawnLayout`, `cell_at`
- `crates/tui_pane/src/tiles/flow_layout/mod.rs` — drop the dead-code expectation, widen read items
- `crates/tui_pane/src/pane/frame.rs` — `pub(crate)` scratch pre-style helper shared with window drawing
- `crates/tui_pane/src/tiles/mod.rs` — re-exports, module doc
- `crates/tui_pane/src/lib.rs` — re-exports `FlowTileCells`, `draw_tile_flow`
- `crates/tui_pane/CHANGELOG.md` — `### Added` entry
- `crates/tui_pane/src/tiles/draw/tests/flow_tests.rs` — new: flow drawing tests

**Workers:** 1 writer + 1 tester — `grid.rs` and `draw.rs` change together and cannot split; the tests are their own file.
- `impl` — `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/flow_layout/mod.rs`, `crates/tui_pane/src/pane/frame.rs`, `crates/tui_pane/CHANGELOG.md`; hub: `crates/tui_pane/src/tiles/mod.rs`, `crates/tui_pane/src/lib.rs` (re-exports)
- `test` — `crates/tui_pane/src/tiles/draw/tests/flow_tests.rs`: every case in Spec item 9, written against the signatures in items 2–3

**Constraints from prior phases:**
- Phase 1 built `crates/tui_pane/src/tiles/flow_layout/` (`mod.rs` + `split_builder.rs`, declared `mod flow_layout;` in `tiles/mod.rs`, not re-exported): `FlowContent<Id> { Group(Id), Empty(usize) }` with `From` into `TileContent<Id>`; `TileRows<Id>::new(content, content_rows)` and `framed()` (content + `TILE_ROWS_READOUT_HEIGHT` + `TILE_BORDER_ROWS`, at least `MIN_TILE_HEIGHT`); `ContentWindow { first_row }`; `PieceSpan { column, top, framed }`; `SplitPiece { at, window, content_rows: NonZeroU16 }`; `SplitTile<Id>` (head plus a non-empty run of continuations, contiguous windows, built only by the packer's `SplitBuilder`; `head()`, `continuations()`); `FlowPiece<Id> { Whole { tile, at }, Split(SplitTile) }`; `UnseenTiles { None, Trailing(NonZeroUsize) }`; `FlowLayout::pack(bands: &[u16], tiles: Vec<TileRows<Id>>) -> FlowLayout<Id> { pieces, unseen }`; `shown_content_rows(framed)`. Items are private behind `#![cfg_attr(not(test), expect(dead_code, …))]`, which this phase removes.
- Pack units: framed rows on shared borders; a band is a column's rows below the summary; a piece at `top` ends at `top + framed` and the next starts at `top + framed - 1`; each column's last piece reaches the band foot; a tile continues across as many columns as it needs; the last column keeps every tile at `MIN_TILE_HEIGHT`, clips only its last piece, and reports tiles that do not fit even at the floor as `Trailing(n)`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` green with every Spec item 9 case; `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` green with no snapshot edited; `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` green (it still calls `draw_tile_grid`); `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` clean; the CHANGELOG entry is present.

### Phase 3 — tui_pane flow motion  · status: todo

**Depends on:** Phase 2

**Blocked by:** tile-fixes — the same merge as phase 2.

#### Work Order

**Goal:** flow pieces move with the grid's existing motion: a piece that moves or resizes slides, and a continuation opens and closes the way a tile does, with no doubled or torn rows at any point of the clock.

**Spec:**
1. Re-read `tiles/grid.rs` motion after the merge (`Transition`, `Step`, `progress`, `eased`, `column_bands`, `moving_cell`, `wrapping_cell`, `BandPieceMotion`, `place_band_pieces`, `TileSettings::animation_millis`) and `tiles/draw.rs` (`draw_tile_flow` from phase 2). Jumping is not acceptable, because cargo-handler's tiles already slide (ruling 3).
2. Flow pieces are keyed by the tile's cell content and a part: `enum FlowPart { First, Continuation(NonZeroUsize) }`, where `First` is a `Whole` piece or a split's head and `Continuation(k)` is its k-th continuation. A tile becoming split keeps its `First` piece and gains continuations; a split shrinking or rejoining keeps `First` and loses its highest continuations.
3. `TileGrid` keeps the flow geometry its last frame drew. When a newly packed geometry differs (any piece's column, top or framed rows, a continuation forming or leaving, a tile joining or leaving), a flow transition starts on the grid's clock, timed by `TileSettings::animation_millis`, from the geometry drawn at that frame to the new one. A change arriving mid-motion starts the next transition from the geometry drawn at that instant, so motion never jumps. A focus change alone moves nothing, since pack ignores focus.
4. Motion per piece, through the existing band machinery so dividers stay one line per boundary: present before and after in the same column → `BandPieceMotion::Resident` (slides and resizes); `First` changing column → `wrapping_cell`; a `Continuation` appearing → `Entering`, as a tile opens; a `Continuation` leaving → `Leaving`, or `ClosingWithColumn` when its column closes, as a tile closes; tiles joining or leaving use the grid's opening and closing motion. Feed both flow geometries to `place_band_pieces`. If it needs a `Grid` for before and after, build them from the flow columns and bands, or factor its band topology out so both callers share it; never write a second divider placement.
5. A split piece in motion still draws its window (phase 2 item 6), clipped by its moving frame.
6. Tests go in `crates/tui_pane/src/tiles/draw/tests/flow_motion_tests.rs`, declared inside `draw.rs`'s `mod tests` as `mod flow_motion_tests;`, drawing through `drawing_at` (or its post-merge equivalent) at raw progress 0, ¼, ½, ¾ and settled. Required cases:
   - A split forming (a tile's demand grows past its column's rows): at every sample each content row of that tile appears at most once across its pieces, and no column shows two stacked border rows.
   - A split leaving (the demand shrinks back): the same two checks, and the settled frame has no continuation.
   - A tile joining before a split tile slides the later pieces; no sample jumps a piece by more than its eased step.
   - A focus change starts no flow transition.

**Files:**
- `crates/tui_pane/src/tiles/grid.rs` — flow geometry memory, flow transition, band pieces for flow
- `crates/tui_pane/src/tiles/draw.rs` — `draw_tile_flow` draws the moving geometry; `mod flow_motion_tests;` inside `mod tests`
- `crates/tui_pane/src/tiles/flow_layout/mod.rs` — `FlowPart`, if the key lives with the packer
- `crates/tui_pane/src/tiles/draw/tests/flow_motion_tests.rs` — new: motion tests

**Workers:** 1 writer + 1 tester — the motion is one change across `grid.rs` and `draw.rs`; the tests are their own file.
- `impl` — `crates/tui_pane/src/tiles/grid.rs`, `crates/tui_pane/src/tiles/draw.rs`, `crates/tui_pane/src/tiles/flow_layout/mod.rs`
- `test` — `crates/tui_pane/src/tiles/draw/tests/flow_motion_tests.rs`: every case in Spec item 6, against `draw_tile_flow` and `drawing_at`

**Constraints from prior phases:**
- Phase 1's packer (`flow_layout/`): `FlowLayout::pack(bands, tiles) -> FlowLayout<Id> { pieces, unseen }` with `FlowPiece::{Whole { tile, at }, Split(SplitTile)}`, `SplitTile` a head plus a non-empty run of contiguous continuations (`SplitPiece { at, window, content_rows }`); pure and stateless per call.
- Phase 2's entry: public `FlowTileCells<Id>: TileCells<Id>` with `continued_title(&self, head: Line<'static>) -> Line<'static>` and `draw_tile_flow(buffer, grid, area, growth, contents, cells)` in `draw.rs`; flow geometry on `TileGrid` in `grid.rs`; `DrawnLayout::{Divided, Flowed(..)}` drives `cell_at`; split pieces draw through a scratch window pre-styled by a shared `pane/frame.rs` helper; the per-piece drawing helper is shared with `draw_tile_grid`, whose output is unchanged. Phase 2 drew the settled flow layout each frame, with no motion.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test tui_pane` green with every Spec item 6 case; `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` green with no snapshot edited; `bash ~/.claude/scripts/delegate/verify.sh lint tui_pane` clean.

### Phase 4 — cargo-handler on flow  · status: todo

**Depends on:** Phase 3

#### Work Order

**Goal:** cargo-handler draws its grid through `draw_tile_flow`, so a tile too tall for its column continues in the next under `<name> · <role> · continued`, and short tiles take only their rows.

**Spec:**
1. `crates/cargo-handler/src/constants.rs`: `pub(crate) const CONTINUED_TITLE_LABEL: &str = "continued";` beside `HEADING_SEPARATOR` (≈203), with a doc comment (what a continuation's title adds after its head's).
2. `crates/cargo-handler/src/render.rs`: `impl FlowTileCells<AgentCell> for Cells<'_>` after the `TileCells` impl (≈136–209). `continued_title(&self, head)` returns `head` with one span appended: `format!("{HEADING_SEPARATOR}{CONTINUED_TITLE_LABEL}")` in `Style::default().fg(label_color())`, so `hana · showrunner` continues as `hana · showrunner · continued` with the name's hue and the role's label color kept.
3. `draw_panes` (≈86) calls `tui_pane::draw_tile_flow` with the same arguments in place of `tui_pane::draw_tile_grid` (≈94); its doc comment names `draw_tile_flow`. The test helper `drawn_buffer_in` (≈765) switches the same way, so tests draw what the app draws.
4. `agent_cell.rs` is not edited: `draw_tile_flow` lays each split piece out at its full demand height, so `CellView::fitted` keeps the full view; the compressed view shows only in a whole piece cut in an overfull last column.
5. Existing `render.rs` goldens that pin divided-grid geometry may change, since short tiles no longer take an even share. Update each such expectation to the flow layout and keep every assertion about content, titles, hues and role labels.
6. `crates/cargo-handler/CHANGELOG.md` `## [Unreleased]`: a tile with more rows than its column has continues at the top of the next column, titled `· continued`; tiles after it move along and short tiles take only the rows they need.
7. Tests go in `crates/cargo-handler/src/render/tests/flow_tests.rs`, declared inside `render.rs`'s `mod tests` as `mod flow_tests;`, reusing `boss`, `drawn_buffer_in` and `drawn_grid` through `super::`. Required cases:
   - A two-column body with one agent whose cell needs more rows than its column: the next column's top title reads `<name> · <role> · continued`, and its first content row is the row after the head's last, compared against the same cell drawn at full height into a tall buffer.
   - A short agent's tile takes only its demand, with the column's last tile reaching the foot.
   - The continuation's name span keeps the agent's hue, and ` · continued` is in the label color.

**Files:**
- `crates/cargo-handler/src/constants.rs` — `CONTINUED_TITLE_LABEL`
- `crates/cargo-handler/src/render.rs` — `FlowTileCells` impl, `draw_tile_flow` in `draw_panes` and `drawn_buffer_in`, goldens updated, `mod flow_tests;` inside `mod tests`
- `crates/cargo-handler/CHANGELOG.md` — `## [Unreleased]` entry
- `crates/cargo-handler/src/render/tests/flow_tests.rs` — new: flow tests

**Workers:** 1 writer + 1 tester — the impl, call switch and golden updates sit in `render.rs`; the new tests are their own file.
- `impl` — `crates/cargo-handler/src/render.rs`, `crates/cargo-handler/src/constants.rs`, `crates/cargo-handler/CHANGELOG.md`
- `test` — `crates/cargo-handler/src/render/tests/flow_tests.rs`: every case in Spec item 7

**Constraints from prior phases:**
- Phase 2: `tui_pane::FlowTileCells<Id>: TileCells<Id>` has one required method, `fn continued_title(&self, head: Line<'static>) -> Line<'static>`, called only when `group_title` returned `Some`; `tui_pane::draw_tile_flow(buffer, grid, area, growth, contents, cells)` takes the same arguments as `draw_tile_grid`. tui_pane draws split pieces itself (full-height scratch layout, rows copied per piece), so the consumer implements no window drawing. Each piece shows its own readout. A tile may continue across several columns, and every continuation carries the continued title.
- Phase 3: flow pieces slide, and continuations open and close like tiles, on the grid's clock; nothing in cargo-handler drives that motion.
- `cell_at` answers against flowed pieces once `draw_tile_flow` has drawn, so clicks keep working unchanged in `app.rs`.

**Acceptance gate:** `bash ~/.claude/scripts/delegate/verify.sh test cargo-handler` green with every Spec item 7 case; `bash ~/.claude/scripts/delegate/verify.sh lint cargo-handler` clean; `bash ~/.claude/scripts/delegate/verify.sh test cargo-tile` green.

### Phase 5 — Live check  · status: todo

**Depends on:** Phase 4

#### Work Order

**Goal:** the real natedev grid at the user's terminal size shows every row of natedev and hana, `· continued` titles on continuations, compact short tiles, and a split that slides as it forms and leaves.

**Spec:**
1. The unit director installs the checkpoint binary and runs cargo-handler on natedev at the user's terminal size, with the Mac shown.
2. Check: the natedev and hana tiles show every content row across their pieces, each continuation's title reads `<name> · <role> · continued`, short tiles hold only their rows with no gap at a column's foot, and every readout reads green unless the grid is overfull.
3. Make a split form and leave (a tile's demand crossing its column's rows, or a terminal resize) and confirm the pieces slide with no doubled or torn rows.
4. Send the shots to the showrunner, and record the result here.

**Files:**
- `docs/cargo-port/remove-running-section-handler-overflow.md`

**Workers:** none — the unit director runs the live check.

**Acceptance gate:** shots of the live grid show every check in Spec item 2, a split forming and leaving slides cleanly, and the showrunner has the shots.
