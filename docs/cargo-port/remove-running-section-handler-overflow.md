# handler-overflow

> **Status: DESIGN — approved by the showrunner 2026-10-15.** A tile whose content needs more rows than its column has continues at the top of the next column, titled `<name> · <role> · continued`. Tiles after it move along, and short tiles take only the rows they need.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-15, the user (via the showrunner): "if a content row can't fit everything such as natedev with 42 rows - it should push hana down … natedev should show everything - cells that need to should be able to overflow into the next column - showing a - continued in the title - like "hana - showrunner - continue" … when we get to cells that don't need much they can give some of their rows back". Screenshots: the natedev tile wants 43 rows and gets 32; the hana tile wants 84 and gets 43; both are cut off.

## Why tui_pane

Only the grid can move later tiles and hand rows back. Today `shares_with_row_usage` (`tiles/grid.rs`) splits each column's rows evenly, lends spare rows to short tiles, and holds rows sticky (`HeldCellLayout`); a tile is always one rectangle. cargo-handler alone could only fake a continuation with invented ids while the grid kept splitting evenly. So the layout lives in tui_pane as a second, opt-in layout. cargo-tile does not call it and keeps its behavior and snapshots.

## Design

**Opt-in by entry point, not by flag.** A consumer gets flow layout by calling `draw_tile_flow(…, cells: &impl FlowTileCells<Id>)`. `FlowTileCells<Id>: TileCells<Id>` adds the two methods a continuation needs:

```rust
fn draw_window(&self, buffer: &mut Buffer, content: &TileContent<Id>, inner: Rect, ground: Color, window: ContentWindow);
fn continued_title(&self, id: &Id) -> Option<Line<'static>>;
```

A consumer that cannot draw from a row offset cannot ask for overflow, because it cannot reach the entry point. A boolean on `TileGrowth` would let "overflow on, offset drawing missing" compile, so there is none.

**Pieces as types.** A new `tiles/flow.rs` packs tiles in order, top to bottom, column by column, each at its measured demand (never below `MIN_TILE_HEIGHT`):

```rust
enum FlowPiece<Id> { Whole(TileRows<Id>), Split(SplitTile<Id>) }
struct SplitTile<Id> { head: TileRows<Id>, continued: ContinuedRows }  // built only by TileRows::split_at
enum ContentWindow { Whole, From { first_row: u16 } }
```

`SplitTile` is built only by `split_at`, which takes the column's remaining rows and returns the head plus a continuation starting at exactly the head's last content row. So a continuation without its head, or with a gap or overlap, cannot be built. The head draws `group_title`; the continuation draws `continued_title` and `ContentWindow::From { first_row }`. Each piece shows its own readout.

**Rules.**
- A tile splits only when the remaining rows in its column fit at least `MIN_TILE_HEIGHT`. Otherwise it starts whole in the next column. A tile splits at most once.
- Rows left at the bottom of a column go to that column's last piece, so no gap is drawn. Short tiles hold only their demand: rows are given back each frame, and nothing is sticky.
- Column count and widths come from the existing `columns`/`fills`. The summary keeps its slot at the top of column 0.
- When the whole grid still cannot fit, the last column's last piece is cut, and its readout shows the shortfall in red, as today.
- Motion: `draw_tile_grid` already animates tiles in cargo-handler (the motion clock and `TileDrawing` band pieces in `tiles/grid.rs`, such as `wrapping_cell` and `place_band_pieces`). So flow pieces use the same motion: a piece that moves or resizes slides, and a continuation appears and leaves the way a tile opens and closes.

**Measured once.** Flow reads each tile's demand from tile-fixes' per-frame measurement (`FrameCellRowAnswers`), so a tile is measured once per frame, not again for flow.

**cargo-handler.**
- `Cells` implements `FlowTileCells`. `draw_window` renders the full cell at its full height into a scratch `Buffer` and copies rows from `first_row` into the piece.
- Each piece then gets its full demand, so `CellView::fitted` keeps the full view. The compressed view shows only in a clipped last piece.
- `continued_title` is `group_title` plus ` · continued` in the label color.
- `terminal.rs` calls `draw_tile_flow`.

## Coordination

tile-fixes is mid-rewrite in `tiles/grid.rs` and `tiles/draw.rs`: the measured-once types, `prepare_frame`, and the dropped auto view. Phase 1 is a new file (`tiles/flow.rs`) with no edits to those two files. Phases 2–3 edit `draw.rs`/`grid.rs` and are claimed `--after tile-fixes`; they start once tile-fixes' measured-once work is checkpointed, and build on its types.

## Phases

1. **tui_pane flow packer** (`tiles/flow.rs`, pure): `FlowPiece`, `SplitTile`, `ContentWindow`, `split_at`, and `pack(columns, column_rows, demands)`. Tests: natedev 43 rows in a 32-row column splits 32/11, and the rest move down; short tiles give rows back; a split that would leave less than the minimum moves whole; an overfull grid clips the last piece.
2. **tui_pane flow entry** (after tile-fixes): `FlowTileCells`, `draw_tile_flow`, titles and readouts per piece, CHANGELOG. Tests: a continuation's cells read the head's next rows; cargo-tile snapshots unchanged.
3. **tui_pane flow motion** (after tile-fixes): flow pieces move with the grid's existing motion. A moved or resized piece slides, and a continuation opens and closes like a tile. Tests: snapshots across the motion clock show no doubled or torn rows while a split forms and while it leaves.
4. **cargo-handler on flow**: `draw_window` (scratch-buffer copy), `continued_title`, and the switch in `terminal.rs`. Tests: a two-column fixture with a split tile reads `· continued` and the right rows.
5. **Live check**: real natedev grid at the user's size: natedev and hana show every row, `· continued` titles show, short tiles are compact, and a split forming or leaving slides. Shots go to the showrunner.

## ETA

Phase 1 now, about 1.5 h. Phases 2–5 about 6 h once tile-fixes checkpoints its grid work; the showrunner sends word when it merges. If that lands within 2 h, done about 03:00 EDT.

## Rulings (showrunner, 2026-10-15)

1. Opt-in by entry point and the `FlowTileCells` trait, not a flag.
2. A column's leftover rows go to its last piece.
3. Pieces use the grid's existing motion; jumping is not acceptable, because cargo-handler's tiles already slide.
4. An overfull grid clips the last piece with the red readout, as the last resort only.
