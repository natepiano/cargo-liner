# handler-enh

> **Status: IMPLEMENTATION PLAN — phased, delegate-ready.** A summary that spans several tile columns reads as one unbroken pane: no column divider is drawn through it.

> **Production: remove-running-section** — unit `handler-enh-unit`; production doc `docs/cargo-port/remove-running-section-production.md`

## Source

2026-10-14, the user, relayed by the showrunner: "while it figured out that it needed to show the summary across two tiles - it still draws the column divider line - obscuring actual text in the summary rows that i don't want obscured - have the handler enh fix this first".

The showrunner's ruling: when one pane spans tiles, nothing is drawn between them. The summary rows read as one unbroken line, and each column keeps its usual spacing. Gate: a render test at the live width, with a spanning summary, that asserts no divider cell inside a summary row, and a shot of the live handler.

Shot: the live handler at 429×90 with `tiles.widen_summary = true`. The summary spans two columns, and the line between them overwrites the directory column, so "~/rust │ hana_catalyst" and "direct│ry" appear.

## Cause

`tui_pane::tiles::draw::draw_placements` (`crates/tui_pane/src/tiles/draw.rs:325`) adds every column band's whole frame to `GridLines` (`grid_lines.add(PaneFrame::new(column))` for each `drawing.column_bands`). Each band runs the full height of the grid. When the summary is widened across bands (`summary_span > 1`, `grid.rs:3440`), the vertical edge two bands share runs straight through the summary's interior. The summary's contents are drawn first and the lattice goes over them, so that line covers one character of every summary row. The fault is in `tui_pane`, not in cargo-handler: cargo-handler draws the summary at the full spanned width.

## Delegation Context

- **Worktree:** `/home/natepiano/rust/cargo-liner-handler-enh`, branch `remove-running-section-handler-enh`.
- **Key files:**
  - `crates/tui_pane/src/pane/frame.rs`: `GridLines` (400), with `add` (437), `add_rule` (508), `render` (559), `mark` (692), `cell_at` (703), `run` (726) and `glyph` (738). `Sides`/`Side` hold up, right, down and left bits.
  - `crates/tui_pane/src/tiles/draw.rs`: `draw_placements` (325). The `TileContent::Summary` arm adds the summary's `line_frame`. The test helper `is_lattice_glyph` and the test `a_grid_too_wide_for_its_window_draws_the_summary_alone` (≈4843) already assert that no divider appears inside a summary-only frame.
  - `crates/cargo-handler/src/render.rs`: the tests module (≈230) with `laid_out_app`, `boss`, `drawn_buffer` (80×24 body) and `Cells`.
- **Overlap:** `crates/tui_pane` is held by tile-fixes (reservation `01a13687-deb4-7be3-aa77-c595d43377af`). The showrunner answered `--before`: this unit lands first, and tile-fixes, which has uncommitted edits in `frame.rs` and `draw.rs`, resolves any conflict. Edits stay on the named lines.
- **Build / test / lint:** `bash ~/.claude/scripts/delegate/verify.sh {check,test,lint} tui_pane cargo-handler`
- **Invariants:** tests run with nextest. Format with `cargo +nightly fmt` only. No test takes a second. Shared borders stay one line wide, and their crossings still resolve through `glyph`.

## Phases

### Phase 1 — A spanning summary has no divider through it  · status: pending

**Depends on:** none

#### Work Order

**Goal:** no lattice cell is drawn inside the summary's interior at any width, and its border joins the column lines below it cleanly: `─` along its top where a band edge used to meet it, `┬` along its bottom where the column line carries on below.

**Spec:**

1. **`GridLines::open(&mut self, rect: Rect)`** in `frame.rs` (pub, next to `add_rule`): every cell strictly inside `rect` loses all of its sides. On `rect`'s own edges, each cell loses the side that points inward (Down on the top row, Up on the bottom row, Right on the left column, Left on the right column), and the corners stay as they are. Its doc says it is for one pane spanning neighbours' bands, so nothing crosses it.
2. **`draw_placements`**: once all lines are added, and before `render`, call `grid_lines.open(rect)` for the summary piece's `line_frame.rect()`. Do it only when `has_visible_body_row` holds for that piece. Collect the rect in the loop and open it after the loop, so that no band or piece added later puts a line back.
3. **Tests:**
   - `frame.rs`: two side-by-side panes with a third pane spanning both above them. After `open` on the spanning pane, its interior is blank, its top is unbroken `─`, and its bottom carries `┬` where the column line leaves it.
   - `draw.rs`: a widened summary across two bands (`widen_summary: true`, stub cells wide enough to span). No `is_lattice_glyph` cell sits inside the summary's inner rect.
   - `cargo-handler/src/render.rs`: the gate test at 429×89 (the live body). `widen_summary` is on, there are enough agents with long directories that the summary spans two columns, and it is settled. Assert that the summary spans more than one band and that no row of the summary's inner rect holds a lattice glyph. The test reads the summary's rect off the drawn buffer's `┌ summary` corner and its bottom border, not off any private API.
4. **Live shot:** after install, take a screenshot of the running handler (429×90). The summary rows are unbroken.
5. **CHANGELOG** for both crates, under `## [Unreleased]` / `### Fixed`.
