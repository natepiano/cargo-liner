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
mark part of a list's rows as pinned so only the remainder scrolls.

Both consumers are the **same** pattern: one cursor over the full row list, one
contiguous scrolling band, the rest pinned. Neither has multiple scroll regions
competing for keyboard or wheel input. So the abstraction is the light one — a
pinned-head / pinned-tail split with a single cursor — not a heavier
multi-region pane with per-region focus routing (see Decisions → D3).

## Non-goals

- No multi-region input routing. One cursor, one scrolling band per pane.
- No new widget type. The framework owns scroll state and cursor math; each pane
  owns its pixel geometry — it splits its inner rect and computes the band's
  visible height, then renders its own rows (CPU via `Layout` constraints,
  Package via a `Paragraph` at manual y-offsets).
- No recursive / nested scroll regions. The pane-grid stays a one-level flat
  model. Recursive layout plus nested scroll windows are a future escalation,
  gated on a real multi-independent-region consumer (Decisions → D3).
- No change to multi-GPU detection — separate work tracked below. (The GPU block
  does collapse from two rows to one this release; see Phase 3.)

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
keeps the cursor on screen." That logic is duplicated three ways:

- `src/tui/settings.rs:126` — `settings_scroll_offset(selected_line, visible_height, line_count)`
- `src/tui/sccache/render.rs:307` — `keep_visible_scroll_offset(selected_line, visible_height, line_count)` (identical signature and job)
- `src/tui/panes/package.rs:158` — `detail_column_scroll_offset(focus, focused_output_line, visible_height) -> u16` (used by git + package detail columns; `u16`, focus-gated, **no upper clamp**)
- the table-backed panes sidestep it by mirroring ratatui `TableState::offset()` back into the viewport

### CPU pane (`src/tui/panes/cpu.rs`)

`CpuPanelLayout::new` (`cpu.rs:182`) builds one vertical `Layout` of
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

### Package pane (`src/tui/panes/package.rs`)

The stats column stacks two `render_stat_section` calls (`render_stat_section`,
`package.rs:793`): Structure at `area.y`, then a "Tests" rule, then Tests below
it. Each section renders all its rows into a fixed-height `section_area` via
`Paragraph::new(lines)` with no scroll offset; overflow is clipped by the
terminal. Rows are `Vec<(&'static str, usize)>` (`pane_data/mod.rs`). The pane
has one `Viewport`; the cursor flows Description → Fields → `Structure(i)` →
`Tests(i)` as a single sequence (`PackageRow` enum, `pane_data/mod.rs:563`). The
Tests rows (unit / integration / doctest counts) already exist on this branch.

## The abstraction

A validated `Band { head, tail, len }` value type describes the partition:

- `head` — leading logical rows that never scroll.
- `tail` — trailing logical rows that never scroll.
- `len` — total row count, owned by the band so its methods can't drift from a
  separately-passed length. The pane constructs it from `viewport.len()` each
  frame: `Band::new(head, tail, viewport.len())`.
- the constructor returns `Option<Self>` — `None` when `head + tail > len`, so an
  empty/negative band cannot be built (Decisions → D3).
- `band_len()` returns `len - head - tail`; the band is the rows in
  `[head, len - tail)`. `contains_cursor(pos)` tests that range;
  `band_local_cursor(pos)` returns `Some(pos - head)` in-band and `None` on a
  pinned row, so the offset clamp is only reachable when the cursor is in the
  band.

The band is composed from pieces the framework already has, kept pane-side per
the resolved decisions:

- **Offset (pane-owned, D2).** The pane computes `band_offset` with the Phase 1
  clamp over the band-local cursor —
  `keep_visible_scroll_offset(pos - head, band_visible, band_len)` — but only
  while `pos` is inside the band; on a pinned row it holds the prior offset.
- **Visible height (pane-computed, D1).** `band_visible` is the band rect's
  height, which the pane derives from its own layout using saturating
  arithmetic. When it is 0 (terminal too short) the pane skips the band slice
  and the affordance rather than feeding a zero-height rect to
  `render_overflow_affordance`.
- **Affordance.** `ViewportOverflow` and `render_overflow_affordance` already
  take arbitrary rects/counts. Add a named `ViewportOverflow::band(band_len,
  band_offset, band_visible, band_cursor)` constructor (a zero-cost alias for
  `new` whose label math is already partition-agnostic) so both panes build band
  overflow facts the same typed way instead of hand-passing the four band
  numbers and risking a drift between CPU and Package. The pane draws the label
  on the band rect — band pages, not full-list pages.
- **Cursor (unchanged).** The cursor is one sequence over the full list
  (`pos` in `0..len`). `up`/`down`/`page_*`/`home`/`end` and `pos_to_local_row`
  operate on the full list and never need the band; only the band's *offset*
  responds to the partition.

Both consumers fit:

- **CPU:** `head = 1` (aggregate), `tail = 4` (System / User / Idle / GPU),
  computed from a helper, not a literal, so multi-GPU can grow it. `band_visible`
  is measured from the cores rect directly, so the pane never reconciles
  per-row heights — the separator rules and the (now one-row) GPU block are part
  of the pinned rects, not the cores rect. The Phase 3 GPU collapse is a UI
  simplification, not a precondition for the math.
- **Package:** `head = Structure-and-above`, `tail = 0`. The pane's
  `band_visible` subtracts the pinned section's pixel height — Structure rows
  plus the "Tests" rule and spacer, which are non-selectable separators the pane
  already renders.

**Placement of the band (D6).** `Band` is a pure pane-side value type;
`Viewport` is untouched. Storing `Option<Band>` on `Viewport` would only be to
let `Viewport::overflow()` report band facts — but `overflow()` is built from the
full-list `len`, `scroll_offset`, and `pos`, so it *cannot* report band facts
anyway; the pane builds `ViewportOverflow::band(...)` regardless. Pane-side also
avoids a `clear_surface()` reset and an unused field on every bandless pane.

## Plan

Each phase ends green: `cargo build && cargo +nightly fmt && cargo mend && cargo
nextest run`, then `cargo install --path .` once the feature is working. Each
phase is a single reviewable commit.

### Phase 1 — Lift the scroll clamp into the framework

Add one `keep_visible_scroll_offset(cursor, visible_rows, len) -> usize` free fn
to `tui_pane`'s viewport module. Replace three call sites:
`settings_scroll_offset` (`settings.rs:126`), the sccache copy
(`sccache/render.rs:307`), and `detail_column_scroll_offset` (`package.rs:158`).
The first two are behavior-identical to the lifted fn (verified across all input
classes); unify on the explicit `line_count <= visible_height` guard. The third
returns `u16`, is focus-gated, and has no upper clamp — keep its
`PaneFocusState::Active` gate and `u16` conversion as thin wrappers at the git /
package call sites; the shared fn stays pure `usize`. Verify the added
`.min(len - visible_height)` is a no-op at those sites (cursor already bounded);
if it ever differs, the clamp is the correct behavior and the old unbounded
result was a latent bug. Unit-test the fn and both detail-column sites. If the
added clamp does change a detail-column result, it lands as a named latent-bug
fix in this same commit, not a phase split. Stands alone, worth doing regardless
of the band work.

### Phase 2 — `Band` value type

Add a validated `Band { head, tail, len }` to `tui_pane`:
`pub fn new(head, tail, len) -> Option<Self>` (`None` when `head + tail > len`),
plus `band_len()`, `contains_cursor(pos)`, and
`band_local_cursor(pos) -> Option<usize>` (`None` on a pinned row). Add the
`ViewportOverflow::band(...)` constructor (D6) so the affordance has one typed
path. Settle the band's home per D6 (pane-side value type, `Viewport`
untouched). Unit tests: `new` returns `None` for `head + tail > len`; `band_len`
for `tail = 0` (Package) and `head + tail` (CPU); `contains_cursor` /
`band_local_cursor` at `head`, mid-band, and the first pinned-tail row (the last
returns `None`); `band_len == 0` and `head + tail == len` (no band);
`ViewportOverflow::band` facts at band top / middle / bottom. One integration
doctest composes `Band` with the Phase 1 clamp —
`keep_visible_scroll_offset(band.band_local_cursor(pos)?, band_visible,
band.band_len())` — so the intended usage is proven before any consumer lands.
No consumer yet; fully unit-tested.

### Phase 3 — Collapse the CPU GPU block to one row

Multi-GPU is deferred and GPU is a single aggregate, so the 2-row GPU block
should be one row. Three coordinated edits in `cpu.rs`:

- Change the GPU `Constraint::Length(2)` in `CpuPanelLayout::new` to
  `Length(1)`.
- Collapse `render_gpu_row`'s two-line `Paragraph` (a "GPU" header line + the
  bar line) into a single line — label and bar inline, matching the System /
  User / Idle rows. Without this the bar line is clipped by the now-1-tall rect.
- Drop `CPU_STATIC_INNER_HEIGHT` from 8 to 7. It feeds `cpu_required_inner_height`
  → `cpu_required_pane_height` → `tiled_row_constraints` (`layout.rs`), so the
  tiled middle-row height auto-tracks; the two test assertions
  (`render.rs:695`, `layout.rs:308`) call `cpu_required_pane_height(12)` on both
  sides, so they auto-track too.

Leave the row-index math alone: `gpu_row = idle_row + 2` skips the non-selectable
separator rule between Idle and GPU (a constraint *position*), not the GPU's
height — collapsing the height moves no index. The separator rules stay. The
selectable count stays `core_count + 5`. Standalone, visible, green on its own.

### Phase 4 — CPU cores band

Three steps in dependency order, one commit:

1. Compute `tail` from a helper (breakdown rows + GPU rows), not a literal, and
   split `CpuPanelLayout` to record pinned-head (aggregate), scrolling-middle
   (cores), and pinned-tail (breakdown + GPU) rects.
2. Drop the hardcoded `set_scroll_offset(0)` in `sync_cpu_pane_state` (else the
   band is overwritten to 0 every frame and never moves). Add a `band_offset` to
   pane state; each frame compute it via the Phase 1 clamp and `Band` only while
   the cursor is in the band (`band_local_cursor` is `Some`); hold the prior
   offset on a pinned row. Compute `band_visible` from the cores rect
   (saturating; skip the band and affordance when 0).
3. Render only the visible core slice (`[band_offset, band_offset +
   band_visible)`) into the middle rect — pushing `row_rects` only for rendered
   cores — and draw the affordance on the cores rect.

Verify the cursor still flows across all rows and clicks on visible cores still
land via `row_rects`.

### Phase 5 — Package Tests band

Set `head` to Structure-and-above, `tail = 0`. Compute the head's pixel height
with the same guard `stats_column_line_count` already uses — the spacer and the
"Tests" rule count **only** when both Structure and Tests are non-empty —
then `band_visible = tests_rect.height - head_pixel_height` (saturating).
`render_stats_column` currently renders both sections at fixed y-offsets in one
pass; split it so the `band_offset` shifts the Tests section only, Structure
stays pinned. Draw the affordance on the Tests rect. In the render path, before
rendering, if a rescan has emptied the Tests section while the cursor sits on a
Tests row, move the cursor to the nearest selectable row (reuse the pane's
existing nearest-selectable logic) so it doesn't strand. Validates the
abstraction against the second consumer; builds directly on the Tests rows
already on this branch, no external dependency. Done when both CPU and Package
scroll correctly. (The cursor recovery reuses the per-frame `set_len` +
`package_nearest_selectable_row` clamp already in `render_package_pane_body`, so
no new recovery code is needed. The original draft pointed at `docs/app-api.md`
for the affordance note, but that doc was replaced on this branch — the design
record lives here instead.)

## Decisions (resolved)

- **D1 — Pinned heights live in the pane.** The pane computes `band_visible`
  from its own layout and passes it in; `Viewport` stays count-only and never
  learns pixel geometry. Pinned rows can be taller than one logical row
  (separator rules; historically the GPU block), so the pixel arithmetic is the
  pane's, the logical-row arithmetic is the framework's.
- **D2 — Band offset lives in the pane.** The pane owns `band_offset` and feeds
  it per frame, matching every existing scrolling pane (settings / sccache /
  git / package / table panes already own their offset and re-set it per frame).
  `Viewport` stays frame-local. CPU drops its hardcoded `set_scroll_offset(0)`.
- **D3 — Validated `Band` struct; flat model.** The encoding is a validated
  `Band { head, tail }` (constructor rejects `head + tail > len`) so the invalid
  state is unrepresentable. `Band` is not a new scrolling engine: the offset, the
  Phase 1 clamp, and the affordance are all reused; the one new idea is the
  pinned-region partition. Considered modelling the band as a nested `Viewport`
  per region (recursive: head / band / tail each a sub-viewport, cursor
  continuity reconstructed via an always-on within-pane `focus_next` + edge
  seeding) and rejected for now: both consumers have exactly one scrolling band
  and a single continuous cursor — not multiple *independently* scrolling regions
  with their own cursors, the only case recursive serves that flat cannot. The
  app is flat everywhere today (`PaneGridLayout` is a one-level grid; focus
  tab-order is derived linearly from it), so flat scroll extends the existing
  model while recursive would introduce the first nesting. Code size is
  comparable, not less, for recursive, and it spreads into the shared
  `FocusedPane<AppPaneId>` type rather than staying contained.
  - **Future direction (not now).** If a consumer needs multiple independent
    nested scroll regions in one pane, make the whole pane-grid recursive (nested
    `PanePlacement` children + recursive tab-order derivation) and pair it with
    nested scroll windows the framework can "turn on" per client. The existing
    grid → tab-order → `focus_next` pipeline is the foundation that escalation
    builds on.
- **D4 — Fold all three clamps into one framework fn** (Phase 1). The git /
  package detail-column sites keep their focus gate and `u16` conversion as
  call-site wrappers; the shared fn is pure `usize`. Verify the added upper
  clamp is a no-op at those sites.
- **D5 — Phase 5 has no external dependency.** The `feature/test-visibility`
  branch in the original draft does not exist; its Tests rows already live on
  this branch. Phase 5 sequences after Phases 1–4 here.
- **D6 — Band placement.** RESOLVED → pane-side; `Viewport` untouched. `Band` is
  a pure value type in `tui_pane`; the pane composes `ViewportOverflow::band(...)`
  itself. `Viewport::overflow()` is built from the full list, so it can't report
  band facts even if it held the `Band`, and pane-side avoids a `clear_surface()`
  reset plus an unused field on every bandless pane. Four of five review lenses
  agreed; the one dissent (store on `Viewport` so `clear_surface()` clears a
  stale `band_offset`) is moot — the offset is recomputed each frame and held
  per-pane, so there is no cross-pane stale state.

## Related, separate work

**Multi-GPU detection.** Today `CpuUsage.gpu_percent: Option<u8>`
(`tui_pane/src/diagnostics/cpu.rs:44`) collapses all GPUs to one number; the
macOS path takes only the first `IOAccelerator` match. Detecting more than one
GPU means changing the field to `Vec<GpuUsage>` and iterating every match.
Independent of subpane scrolling — it can land before or after. It interacts
with the CPU `tail` size: each detected GPU adds a row to the pinned tail. Phase
4 computes `tail` from a helper precisely so this work only changes the GPU row
count in one place rather than a fixed literal.
