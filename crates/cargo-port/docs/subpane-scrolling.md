# Subpane scrolling

## Goal

Let a single pane scroll **one contiguous band of rows** while the rows above
and below it stay pinned and always visible. The cursor remains one sequence
that flows across pinned and scrolling rows alike; only the band moves under it.

Two concrete consumers drive this:

- **CPU pane** — on a many-core machine the per-core rows overflow the pane and
  the bottom rows (System / User / Idle / GPU) silently vanish. The cores should
  scroll; the aggregate line (top) and the breakdown + GPU rows (bottom) should
  stay pinned.
- **Package pane** — the stats column stacks **Structure** (capped target-kind
  counts, never overflows) above **Tests** (unit / integration / doc, soon to
  gain per-feature rows). On a large project like bevy the Tests rows will
  overflow. Structure stays pinned; Tests should scroll.

## Motivation

The framework already owns a scrollable-list model (`Viewport`) and an overflow
affordance (`render_overflow_affordance`, the "▲ n of m ▼" label). Both are
rect-scoped, not pane-scoped — nothing assumes the rect is a full pane inner. So
the mechanics of a scrolling region exist; what is missing is the ability to
mark part of a viewport's rows as pinned so only the remainder scrolls.

Both consumers are the **same** pattern: one cursor over the full row list, one
contiguous scrolling band, the rest pinned. Neither has multiple scroll regions
competing for keyboard or wheel input. So the abstraction is the light one — a
pinned-head / pinned-tail split with a single cursor — not a heavier
multi-region pane with per-region focus routing. Building the heavier version
would over-fit; neither consumer needs it.

## Non-goals

- No multi-region input routing. One cursor, one scrolling band per pane.
- No new widget type. The framework owns scroll state and geometry; each pane
  keeps rendering its own rows (CPU via `Layout` constraints, Package via a
  `Paragraph` at manual y-offsets), same division the framework already uses.
- No change to multi-GPU detection — that is separate work tracked below.

## Current state

### Framework (`tui_pane/src/layout/viewport.rs`)

`Viewport` holds `pos`, `hovered`, `len`, `content_area`, `scroll_offset`,
`visible_rows`. It owns cursor movement (`up`/`down`/`page_*`/`home`/`end`),
hit-testing (`pos_to_local_row`), and the overflow facts (`overflow()` →
`ViewportOverflow`). `ViewportOverflow::new(len, scroll_offset, visible_rows,
cursor)` and `render_overflow_affordance(frame, area, overflow, style)` both
take arbitrary rects and counts — they are already general.

The one piece of scroll math the framework does **not** own is the clamp:
"given the cursor, the visible-row count, and the total length, what offset
keeps the cursor on screen." That logic is duplicated:

- `src/tui/settings.rs:126` — `settings_scroll_offset(selected_line, visible_height, line_count)`
- `src/tui/sccache/render.rs:307` — `keep_visible_scroll_offset(selected_line, visible_height, line_count)` (identical signature and job)
- `git.rs` / `package.rs` compute their own `layout.scroll_offset`
- the table-backed panes sidestep it by mirroring ratatui `TableState::offset()` back into the viewport

### CPU pane (`src/tui/panes/cpu.rs`)

`CpuPanelLayout::new` (`cpu.rs:181`) builds one vertical `Layout` of
`Constraint::Length(1)` per row — aggregate, then N cores, then System / User /
Idle, then a 2-row GPU block — and splits it across `inner`. When `inner` is
shorter than the sum, ratatui squeezes the trailing constraints to zero height,
so the bottom rows disappear with no indication.

Scroll is hardcoded off: `sync_cpu_pane_state` (`cpu.rs:386`) calls
`viewport.set_scroll_offset(0)` every frame. Selectable rows are
`total_selectable_rows(core_count) = core_count + 5`
(Aggregate, N cores, System, User, Idle, GPU). Hit-testing uses recorded
`row_rects` (`pane_impls.rs`), not `pos_to_local_row`, so scrolling the band
only changes which rects get pushed — clicks keep working with no extra code.

### Package pane (`src/tui/panes/package.rs`, branch `feature/test-visibility`)

The stats column stacks two `render_stat_section` calls
(`package.rs:685`–`756`): Structure at `area.y`, then a "Tests" rule, then Tests
below it. Each section renders all its rows into a fixed-height `section_area`
via `Paragraph::new(lines)` with no scroll offset; overflow is clipped by the
terminal. Rows are `Vec<(&'static str, usize)>` (`pane_data/mod.rs`). The pane
has one `Viewport`; the cursor flows Description → Fields → `Structure(i)` →
`Tests(i)` as a single sequence (`PackageRow` enum, `pane_data/mod.rs:563`).

## The abstraction

Model the band as two counts on the existing `Viewport`:

- `pinned_head` — number of leading logical rows that never scroll.
- `pinned_tail` — number of trailing logical rows that never scroll.

Everything between is the scrolling band:

- `band_len = len - pinned_head - pinned_tail`
- `band_visible = visible_rows - pinned_head_height - pinned_tail_height`
- the band's offset is computed over `band_len`, and the band moves **only** when
  the cursor `pos` is inside `[pinned_head, len - pinned_tail)`; on a pinned row
  the band holds still.
- `overflow()` reports facts for the band (band length, band offset,
  band-visible, cursor-within-band), so `render_overflow_affordance` draws the
  "▲ n of m ▼" label on the band rect.

Both consumers fit: Package sets `pinned_tail = 0`; CPU sets `pinned_head = 1`
(aggregate) and `pinned_tail = 4` (System / User / Idle / GPU). The pinned-row
**heights** can differ from their **counts** (CPU's GPU row is 2 rows tall, and
both panes have non-selectable separator rules), so the band-visible math takes
heights, not counts, for the pixel arithmetic while the offset math works in
logical rows.

## Plan

Each phase ends green: `cargo build && cargo +nightly fmt && cargo mend && cargo
nextest run`, then `cargo install --path .` once the feature is working.

### Phase 1 — Lift the scroll clamp into the framework

Add `keep_visible_scroll_offset(cursor, visible_rows, len) -> usize` to
`tui_pane` (free fn in the viewport module, or a `Viewport` method). Replace
`settings_scroll_offset` (`settings.rs:126`) and the sccache copy
(`sccache/render.rs:307`) with calls to it. Pure function, fully unit-testable;
no behavior change for existing panes. This stands alone and is worth doing
regardless of the band work.

### Phase 2 — Band support on `Viewport`

Add `pinned_head` / `pinned_tail` state plus setters, derive `band_len` /
`band_visible`, gate the band offset on the cursor being inside the band, and
make `overflow()` report band facts. Unit tests cover: tail = 0 (Package),
head + tail (CPU), cursor on a pinned row (band holds), cursor crossing into the
band (band follows), and the affordance label at band top / middle / bottom.

### Phase 3 — CPU cores band

Split `CpuPanelLayout` into pinned-head (aggregate), scrolling-middle (cores),
pinned-tail (breakdown + GPU). Persist a band offset across frames (drop the
hardcoded `set_scroll_offset(0)`), render only the visible core slice into the
middle rect, and draw the affordance on it. Verify the cursor still flows across
all rows and clicks still land via `row_rects`.

### Phase 4 — Package Tests band

Set `pinned_head` to Structure-and-above, `pinned_tail = 0`, scroll the Tests
section, and draw the affordance on the Tests rect. This validates the
abstraction against the second consumer before it calcifies. (Lands on the
`feature/test-visibility` line; sequence after that branch and this one
converge.)

## Related, separate work

**Multi-GPU detection.** Today `CpuUsage.gpu_percent: Option<u8>`
(`tui_pane/src/diagnostics/cpu.rs:44`) collapses all GPUs to one number; the
macOS path takes only the first `IOAccelerator` match. Detecting more than one
GPU means changing the field to `Vec<GpuUsage>` and iterating every match.
Independent of subpane scrolling — it can land before or after — but it
interacts with the CPU pinned-tail size, since each detected GPU adds rows to
the pinned tail. Noting it here so the band sizing accounts for a variable GPU
row count rather than a fixed two.
