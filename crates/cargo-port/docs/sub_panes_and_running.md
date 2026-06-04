# Running sub-pane for the Targets pane

Status: complete 2026-06-03 — Phases 1 (box tree + CPU), 2 (detail pane), 3 (Targets switch), and 4 (gate `K` + wheel re-anchor) all shipped
Authored: 2026-06-03 (architecture revised same day to the box-tree model)

## Goal

Stop showing a target's running state inline in the Targets table. Instead, give
the Targets pane a bottom sub-pane titled **Running** that lists every tracked
target currently running — its name, run profile, PID, CPU, memory, and the
path from the home directory down to the project. The sub-pane appears on every
Targets section (Binary / Examples / Bench) whenever anything is running, is
anchored to the bottom of the pane, and grows **upward** as more instances start.

## Current behaviour (being replaced)

`build_target_display_rows` (`src/tui/panes/pane_data/mod.rs`) expands the
target list with per-run rows. `TargetDisplayKind` has four variants:

- `Idle` — no running process
- `Inline(RunningInstance)` — one instance, shown inline as
  ` name (debug) 47% 312 MiB`
- `MultiParent(usize)` — parent row of N>1 instances (` name (2 running)`)
- `Instance(RunningInstance)` — per-instance child row

`K` (`TargetsAction::Kill`) resolves the cursor's display row to PIDs via
`resolve_kill_request`. On an `Inline`/`Instance` row it kills one PID; on a
`MultiParent` row it kills **all** of that target's instances at once.

All of this inline machinery is removed by this work.

## Target behaviour

### Layout

The Targets table renders as before but with no inline running state — one row
per target in the current section. Below it, when anything is running, a
**Running** sub-pane is rendered, sharing the pane's left/right/bottom borders
and separated by a `├─ Running (N) ─┤` divider whose tee glyphs merge into the
side borders. The divider rises as the running list grows.

The sub-pane has a column-header row and one data row per running **instance**
across all tracked targets (not only the current section). Columns, left to
right: **Target · Profile · PID · CPU · MEM · Path**. Path is last so CPU/MEM
stay aligned at a fixed left edge regardless of path length.

Single instance running:

```
┌─ Targets: Examples (3) ────────────────────────────────────────────────────┐
│ Target            Source                Kind                                │
│ custom_app_name   bevy_window_manager   example                             │
│ custom_path       bevy_window_manager   example                             │
│ restore_window    bevy_window_manager   example                             │
│                                                                             │
│                                                                             │
│                                                                             │
├─ Running (1) ───────────────────────────────────────────────────────────────┤
│ Target         Profile  PID     CPU   MEM       Path                        │
│ custom_path    debug    48213    47%  312 MiB   ~/rust/bevy_window_manager   │
└───────────────────────────────────────────────────────────────────────────────┘
```

Three instances (the same target run twice plus a second target; divider has
climbed two rows; newest pinned to the bottom):

```
┌─ Targets: Examples (3) ────────────────────────────────────────────────────┐
│ Target            Source                Kind                                │
│ custom_app_name   bevy_window_manager   example                             │
│ custom_path       bevy_window_manager   example                             │
│ restore_window    bevy_window_manager   example                             │
├─ Running (3) ───────────────────────────────────────────────────────────────┤
│ Target         Profile  PID     CPU   MEM       Path                        │
│ custom_app_name debug    48213    12%  201 MiB   ~/rust/bevy_window_manager  │
│ restore_window  cargo     5120    88%  1.2 GiB   ~/rust/other_game           │
│ custom_path     release  48555    47%  312 MiB   ~/rust/bevy_window_manager  │
└───────────────────────────────────────────────────────────────────────────────┘
```

### Behaviour rules

- **Per-instance kill only.** `K` sends `SIGTERM` to the single PID of the
  selected Running row. The "kill every instance of this target" path is gone —
  each instance is its own row and is killed individually.
- **One continuous cursor.** Down past the last table row moves into the Running
  rows (top to bottom); up from the first Running row returns to the table.
- **Height cap.** The Running sub-pane grows upward but never exceeds 80% of the
  pane's inner height; the table keeps at least 20%. Past the cap the Running
  list scrolls like any other list.
- **Newest at the bottom.** New instances append at the bottom; older instances
  sit above.
- **`K` only on a running row.** The Kill hint/binding is shown and enabled only
  when the cursor sits on a Running row; on table rows it is hidden and inert.
- **Present whenever anything runs**, on every Targets section. When nothing is
  running the sub-pane is gone and the table reclaims the full height.

## Architecture

### Guiding principles

- **The model mirrors what's on screen** — boxes inside boxes.
- **The fewest types that capture it** — two small enums and one small struct,
  plus the existing `Viewport` reused for the highlighted row.
- **The layout reads like the screen** — see the three panes written out below.

### One box tree per pane

A pane is boxes inside boxes. That is what is on screen, so that is the model. A
small tree in `tui_pane` describes it, and each pane rebuilds its tree every
frame to mirror its current contents.

Three things are kept apart:

- **Where things sit** — the tree.
- **The highlighted row and scrolling** — the existing `Viewport` holds the one
  highlight (`pos` / `len` / hover / navigation / `selection_state`, all
  unchanged); each scrolling box keeps its own offset, held by the pane across
  frames.
- **Drawing** — the pane draws each box's rows (and any title / header lines)
  into the rect the tree hands it.

The tree, in the fewest types:

- A box (`Region`, name provisional) is one of: a **list of rows**, a **stack**
  of boxes (top to bottom), or **columns** of boxes (side by side).
- Each box has a **size**: `Fixed` (exactly its rows — never scrolls), `Fill`
  (take the room left over, scroll if its rows don't fit), or `Cap(percent)`
  (grow to fit its rows but stop at `percent` of the parent, scroll past that).
  The Running box is `Cap(80)`.
- A list-of-rows box may reserve a line or two of chrome on top (a title
  divider, a column header). The tree only leaves room; the pane draws them.

That is two enums — `Region` and `Size` — and one small struct the layout pass
returns (`Placed`: a box's rect plus its scroll offset). The single highlight is
one number that walks every selectable row in tree order (a stack's children top
to bottom, columns left to right). The layout maps that number to "this box,
this row," scrolls that box to keep it in view, and leaves the other boxes where
they were. Kill, the `K`-only-on-a-running-row rule, and the title count all just
ask which box the highlight is in.

The three panes, written the way they read on screen:

```text
// CPU
Stack[
  rows(1, Fixed),           // aggregate
  rows(cores, Fill),        // cores — scrolls
  rows(3, Fixed).rule(),    // System / User / Idle, with a rule above
  rows(1, Fixed).rule(),    // GPU, with a rule above
]

// Detail pane
Columns[
  rows(metadata, Fill),                       // left column — scrolls
  Stack[
    rows(structure, Fixed),                   // Structure — pinned
    rows(tests, Fill).title("Tests"),         // Tests — scrolls
    rows(cratesio, Fixed).title("crates.io"),
  ],
]

// Targets
Stack[
  rows(table, Fill),                                 // the table
  rows(running, Cap(80)).title("Running").header(),  // the Running box
]
```

This replaces `Band` and the two byte-identical scroll routines
(`cpu_band_offset`, `tests_band_offset_for`). `Band` is deleted once the CPU and
detail panes are on the tree. The work also advances the deferred plan to pull
the generic ratatui pane code into its own crate boundary.

### Data model changes

- **Newest-at-bottom ordering.** `RunningTargets` rebuilds its `HashMap` every
  tick and sorts each key's instances by PID — no first-seen information. Add a
  persisted `pid -> Instant` first-seen map on the tracker (it already persists
  `system` across ticks and `tick()` already takes `now`): insert on first
  sight, drop on exit. The global Running list sorts by first-seen ascending so
  the newest instance is the bottom row.
- **Member-relative path** (per D1). `home_relative_path()`
  (`src/project/paths.rs:180`) turns `/Users/x/rust/foo` into `~/rust/foo`.
  `ProjectTargetSlice` (`src/tui/running_targets.rs`) carries only `target_dir`,
  so thread each running target's member directory (the project root that owns
  the binary, not the shared workspace root) from the call site into the
  snapshot, exposed per Running row. Two members with the same binary name then
  show distinct paths (e.g. `~/rust/bevy_window_manager/foo`).
- **`RunningRow`** — `{ name, profile, pid, cpu, memory, display_path,
  first_seen, create_time }`, built by a global `build_running_rows(snapshot)`
  that flattens every tracked key (not the current section), sorted
  newest-at-bottom. `first_seen` (our first observation) drives the ordering;
  `create_time` (the process's actual start, from sysinfo) drives the kill-time
  validation (D3) and the confirm-dialog start-age (D4).

### Kill behaviour change

`resolve_kill_request` reads which box the highlight is in. When it is in the
Running box, the row within that box selects one `RunningRow`, producing a
`KillRequest` with a single PID. The `MultiParent` all-PIDs branch and the
`TargetDisplayKind` variants it depended on are deleted. The confirm dialog
("Send SIGTERM?", `src/tui/render.rs`) keeps its current rendering; its body
always shows one PID now. `ConfirmAction::KillTarget`'s doc comment is updated to
drop the multi-instance wording.

After a kill the Running list shrinks by one: clamp the highlight and, when the
list becomes empty, the Running box has no rows, so the highlight falls back into
the table box.

### `K` hint gating

The `Shortcuts` trait already exposes `visibility(&self, action, ctx)`
(`tui_pane/src/keymap/shortcuts.rs:53`), defaulting to Visible. The
`TargetsPane` impl (`src/tui/integration/framework_keymap.rs`) overrides it so
`Kill` is Visible only when the highlight sits on a Running row —
`running_cursor_pid().is_some()` is that fact. No `state()` override: a Hidden
slot never reaches `state()`, and there is no Visible-but-Disabled case for
`Kill`.

## Files touched

| File | Change |
| --- | --- |
| `tui_pane/src/layout/` (a new sibling of `viewport.rs`) | add the box tree (`Region`, `Size`, layout pass, one-highlight mapping); delete `Band` |
| `src/tui/panes/cpu.rs` | rebuild the CPU layout as a `Stack`; drop `cpu_band_offset` |
| `src/tui/panes/package.rs` | rebuild the detail layout as `Columns` + `Stack`; drop `tests_band` and `tests_band_offset_for` |
| `src/tui/running_targets.rs` | first-seen `pid -> Instant` map; member-relative display path through `ProjectTargetSlice` into the snapshot |
| `src/tui/panes/targets/` (promote `targets.rs` to `targets/mod.rs`) | build `Stack[ rows(table, Fill), rows(running, Cap(80)) ]`; render table without inline running; render the Running box |
| `src/tui/panes/targets/running_subpane.rs` (new) | `RunningRow`, `build_running_rows`, render the Running box |
| `src/tui/panes/pane_data/mod.rs` | delete `TargetDisplayKind::{Inline,MultiParent,Instance}` + expansion; table is one row per entry; single-PID `resolve_kill_request` |
| `src/tui/panes/pane_impls.rs` | `TargetsPane` keeps its `Viewport` (the highlight; its `scroll_offset` is the table box's persisted offset) plus `running_cursor_pid` (also the `K`-gating fact) and hit-test `row_rects` |
| `src/tui/panes/actions.rs` | highlight navigation across the boxes; single-instance kill; the PID-anchor re-derive hooks (nav, click, wheel) |
| `src/tui/integration/framework_keymap.rs` | `TargetsPane` `visibility()` for `Kill` (Hidden off a Running row) |
| `src/tui/app/types.rs`, `src/tui/render.rs` | `KillTarget` doc + confirm body (one PID now) |
| `src/tui/panes/targets/` constants | the 80% cap, sub-pane chrome rows, `MIN_TABLE_ROWS`, column widths |

## Phases

Each phase ends with `cargo build && cargo +nightly fmt`, `cargo nextest run`,
`cargo mend`, and `cargo install --path .`. `Columns` and `Cap` are each added in
the phase their first user appears, so nothing is unused — the layout engine
grows per-phase rather than as one standalone foundation step, because an unused
`Columns`/`Cap` variant would fail the per-phase dead-code gate.

### Phase 1 — box tree + CPU ✅ complete (2026-06-03)

Add the tree to `tui_pane`: `Region` (rows / `Stack`), `Size` (`Fixed` / `Fill`),
the layout pass, the one-highlight mapping, and per-box scroll offsets. Rebuild
the CPU pane as a `Stack` and drop `cpu_band_offset`. `Band` stays (the detail
pane still uses it). No user-visible change. Unit-test the layout (sizes, scroll,
highlight mapping); the CPU pane's existing tests guard the rebuild.

Shipped in `tui_pane/src/layout/region.rs` (`Region`, `Size`, `Rows`, `Placed`,
`place`/`locate`/`total_selectable`) and `src/tui/panes/cpu.rs` (`cpu_region`,
`CpuPanelLayout` rebuilt on the tree; `cpu_band_offset` deleted).

#### Retrospective

**What worked:**
- The tree dropped onto the CPU pane cleanly. The `Fill` box's scroll offset
  reproduces the old `cpu_band_offset` exactly (clamp to keep the highlight
  visible when the cursor is in the box; hold the prior offset, re-clamped,
  otherwise), so the four offset tests port over and pass unchanged against the
  tree.
- Fewest types held: `Region` (`Rows`/`Stack`), `Size` (`Fixed`/`Fill`), one
  `Placed` struct, and the existing `Viewport` for the highlight — no newtypes.

**What deviated from the plan:**
- "No user-visible change" does not hold in the CPU **over-tall** case. With
  cores = `Fill` and "the one `Fill` takes the remainder" (R2/R16), the
  System/User/Idle/GPU rows now pin to the bottom border whenever the pane is
  taller than `7 + core_count` — common, since the CPU pane lives in the
  `Fill(1)` middle row (asserted by `render.rs` `middle_row_expands_to_fit…`).
  Previously they sat directly under the cores with the slack pinned at the
  bottom. The exactly-sized and cramped/scrolling cases stay pixel-identical.
  There is no way to keep the old over-tall look while honoring "exactly one
  `Fill` per stack" + "`Fill` takes the remainder" — the same invariants the
  Targets layout needs. Accepted as the cost of the unified model.
- Added area-clamping in `place` (a box shrinks rather than pushing later boxes
  off-screen in a too-short pane), matching ratatui's prior `Layout::split`
  clamp — not called out in the plan.

**Surprises:**
- The `Rows` payload's row-count field tripped clippy `struct_field_names`
  (it restated the struct name); renamed `rows` → `count`.
- `place` takes `prior_offsets: &[usize]` indexed per leaf box; the CPU pane
  passes `&[0, prior, 0, 0]` and only box 1 (cores) scrolls. The slice already
  supports Targets' two scrolling boxes.

**Implications for remaining phases:**
- Phase 2 must teach `place`/`locate`/`leaves` real nesting. Phase 1's
  `leaves()` only flattens a stack of `Rows` and `debug_assert`s every child is
  a `Rows`; the detail pane's `Columns[ rows(metadata, Fill), Stack[…] ]` needs
  the `Columns` variant plus recursion (sizing a nested stack as a column
  child, then the highlight/offset indexing walking leaves across the nesting).
- Phase 3's (adoption) Targets table is the `Fill` box and already fills its
  rect (blank rows above the Running box) under the Phase 1 semantics — no extra
  work for the table. Running = `Cap(80)` still needs the `Cap` variant, the
  `min(rows + chrome, floor(percent * inner))` clamp, the `Size::cap` ctor
  `debug_assert`, and the `MIN_TABLE_ROWS` floor (R16).

#### Phase 1 Review

- Phase 2 expanded with the engine work it must land first: nesting in
  `place`/`locate`/`leaves`, the `Columns` variant, `Fill` scoped per stack
  node, the leaf-order convention for `prior_offsets`, the metadata column
  keeping `Paragraph`-line scroll, `content_height` = the taller column,
  `.title()`/`.header()` chrome ctors, and the R18 `Band` test port.
- Phase 2's tree corrected: the About/Description block renders above the
  columns, so the pane is `Stack[ description, Columns[…] ]` — two-level
  nesting.
- Former Phase 4 (now Phase 3) expanded: `Cap` clamps outer height via a
  `Size::cap` ctor; `MIN_TABLE_ROWS` reserved before `Cap` takes its share;
  `box_scroll_offset` gains a `Cap` arm whose non-cursor default pins to the
  bottom (newest-at-bottom, R19); kill resolution rebuilt against
  `RunningRow`/box-index; `TargetsPane` gains `running_cursor_pid` plus a
  per-box offset field, with the PID→row remap ordered after the table count.
- Former Phase 5 (now Phase 4): persist the `K`-gating fact on `TargetsPane`
  each frame for `visibility()`/`state()`.
- Decision (approved): former Phase 3 merged into the adoption phase — the data
  model had no consumer until the switch and could not ship clean under the
  dead-code gate. Phases renumbered to four; R15/R20/R22 updated to match.
- Rejected: consolidating the layout-engine work (nesting + `Columns` + `Cap`)
  into a standalone foundation step — unused `Columns`/`Cap` variants would
  fail the per-phase dead-code gate; the engine grows in the phase its first
  user appears, as the Phases intro states.

### Phase 2 — detail pane onto the tree ✅ complete (2026-06-03)

Add `Columns` and nested-stack handling to the layout pass, then rebuild the
detail (Package) pane. The real tree is two levels deep: the About/Description
block renders first (`render_project_description_section`), so the accurate
layout is `Stack[ rows(description, Fixed), Columns[ rows(metadata, Fill),
Stack[ Structure Fixed, Tests Fill, crates.io Fixed ] ] ]` — a `Columns` node
nested in a `Stack`. Preserve current behaviour exactly (description on top,
Structure pinned, Tests scrolls, the cross-column cursor, each column's offset
persisting while the highlight is in the other). Delete `Band`, `tests_band`,
and `tests_band_offset_for`. The detail pane's tests guard it.

Carried from the Phase 1 review — engine work this phase must land before it can
rebuild the pane:

- **Nesting in `place`/`locate`/`leaves`.** Phase 1's `leaves()` flattens a
  single stack of `Rows` and `debug_assert`s no nesting. Teach the pass to size
  a nested stack as a column child and recurse, then index the highlight and
  offsets across the nesting.
- **`Fill` is one-per-stack-node, not one-per-tree.** The detail pane has two
  `Fill` leaves — metadata (left column) and Tests (right column). The "exactly
  one `Fill`" invariant must scope to each stack node, not the flattened leaf
  set Phase 1 currently asserts.
- **Leaf-order convention for `prior_offsets`.** `place` indexes
  `prior_offsets: &[usize]` by flattened-leaf position; define and test the leaf
  order a `Columns` node produces (left column's leaves, then right column's) so
  the pane places its two offsets correctly.
- **Metadata column keeps `Paragraph`-line scroll.** That column scrolls a
  `Paragraph` by rendered output lines (a `PackageRow::Field` can wrap to several
  lines), not by selectable rows. The tree hands it a rect; the column keeps its
  existing `detail_column_scroll_offset` / `Paragraph::scroll` for internal
  scroll. Forcing the tree's row offset onto it would break per-field selection.
- **`content_height` is the taller column.** Keep the multi-column `Viewport`
  divergence (`set_len` = addressable rows, `set_content_height` = rendered
  height): set `content_height` to the taller column so the pager counts stacked
  rows, not the side-by-side total.
- **Chrome ctors.** Tests / crates.io want a titled chrome row, not just a rule;
  add `.title(name)` / `.header()` (chrome-count increments the pane draws)
  alongside the existing `.rule()`.
- **`Band` port-before-delete (R18).** Rewrite `tests_band_offset_for`'s tests to
  assert the tree's offsets before deleting `Band`; `box_scroll_offset` already
  reproduces its in-band / hold-clamped logic, so the port is mechanical.

Shipped in `tui_pane/src/layout/region.rs` (`Columns` variant, recursive
`place`/`locate`/`leaves`, per-stack-node fill invariant, `.spacer()`,
`.lines()`) and `src/tui/panes/package.rs` (`package_region` + `PackageBoxes`;
sections render from `Placed` rects). `Band`, `ViewportOverflow::band`,
`tests_band`, `tests_band_offset_for`, `project_stats_connector_x`,
`StatsColumnLayout`, and `ProjectPanelAreas` deleted;
`PackagePane.tests_band_offset` renamed `tests_scroll_offset`.

#### Retrospective

**What worked:**
- The recursion was small: `place` became a walk with a `PlaceWalk` state
  struct (cursor, prior offsets, running leaf index), a `place_stack` for the
  one-fill-per-stack sizing, and a `place_leaf` shared by every path. The CPU
  tree needed no changes.
- Column widths reuse ratatui `Constraint` directly
  (`Columns(Vec<(Constraint, Region)>)`), so the pane's old
  `Min(20)`/`Length(stats_width)` split is byte-identical — no new width type.
- The per-section `title_y` accumulation in the stats column (structure
  height + spacer + rule + …) deleted wholesale; sections now read
  `Placed.content`/`.chrome`/`.scroll_offset`.

**What deviated from the plan:**
- `.title(name)`/`.header()` were not added. Chrome is count-only and the pane
  draws all chrome, so a stored title would be dead data; a titled rule is one
  `.rule()` row and `.spacer()` covers the section gaps. `.header()` has no
  Phase 2 user — per the dead-code gate it lands in Phase 3 with the Running
  box (which can build its divider + column-header chrome as `.rule().spacer()`
  or grow `.header()` then).
- `.lines(h)` is engine surface the plan didn't name: a box that is N
  selectable rows rendered as `h` screen lines. Needed twice — the description
  block (1 selectable row, H wrapped lines; H known only after the block
  renders, so the pane renders the description first and feeds the returned
  height into the frame's tree) and the crates.io section, whose rendered rows
  can have no flat-list counterpart (worktree-summary data), so the box keeps
  the flat count for the cursor and reserves rendered rows via `lines`.
- "Preserve current behaviour exactly" has the same over-tall exception as
  Phase 1: with column slack, crates.io now pins to the column bottom
  (previously it stacked directly under Tests with the slack below); in
  cramped panes crates.io stays visible and the Tests box shrinks (previously
  Tests extended to the column bottom and crates.io rendered off-screen).
  Exactly-sized columns are pixel-identical. Same cause, same acceptance.
- The shared separator rule (full-width, "Structure" title) is modeled as one
  chrome row on **both** columns' top boxes — the tree reserves the row in
  each column; the pane draws the rule once at full width.

**Surprises:**
- The old trailing-rows `Band` partition (`head = len - test_count`) misfiled
  crates.io rows as Tests rows — the flat list ends `…, Tests…, CratesIo…` —
  so Tests scrolling and its pager misbehaved whenever crates.io rows were
  present. The tree maps each row to its own box; a regression test pins it
  (`package_tree_maps_section_rows_to_their_boxes`).
- The right column's one-fill invariant is data-dependent: Tests is the fill
  when present, otherwise the last present section becomes the fill (its rows
  render from the top of the leftover, so it draws the same as a pinned box).
- `render_package_pane_body` tripped clippy `too_many_lines` after absorbing
  the tree placement; extracted `render_no_project_selected` and
  `sync_package_viewport`.

**Implications for remaining phases:**
- Phase 3's tree is the trivial case: `Stack[ rows(table, Fill),
  rows(running, Cap(80)) ]` — no nesting, no columns. Only `Cap`, its scroll
  arm, and the `MIN_TABLE_ROWS` floor are new engine work.
- The Running box's divider + column-header chrome needs no stored titles:
  reserve two chrome rows and draw them in the pane, as Tests/crates.io do.
- `prior_offsets` indexing by flattened-leaf order is settled and tested; the
  Targets pane's two offsets are `[table, running]` in that order.

#### Phase 2 Review

- Phase 3's engine scope tightened to exactly `Cap`: the `Size::cap` ctor, the
  outer-height clamp where `place_stack` sizes non-fill children, and the
  bottom-pinned `Cap` arm in `box_scroll_offset` — everything else the phase
  assumed (nesting, `Columns`, leaf-order offsets, `.lines()`) shipped in
  Phase 2.
- Phase 3 chrome settled: no `.header()` ctor required — the Running box's
  divider + column-header rows are count-only chrome the pane draws (the
  `render_section_rule` pattern); the Targets table's ratatui `Table::header`
  row is modeled as one chrome row on the table box so the 80% cap and
  `MIN_TABLE_ROWS` floor measure the true inner height.
- Phase 3 data model expanded with work the snapshot does not carry today:
  owning-member manifest-dir resolution from the target name via the
  `TargetEntry` metadata (D1/R20 — the exe path cannot recover it), the
  first-seen map surviving `tick()`'s per-poll rebuild with `kill`/
  `drop_instances` evicting entries, `create_time` via sysinfo `start_time()`
  (extend `ProcessRefreshKind`), and an all-keys accessor on `RunningTargets`
  for the global list.
- Phase 3 adoption expanded: the cursor-split audit now names the run path
  (`handle_target_action`), the anchors (`targets_cursor_entry` /
  `anchor_targets_cursor_to_entry`, `display_row_for_entry` deleted), and the
  title's `cursor_entry` counter; the confirm body is rewritten once in
  Phase 3 with D4's text; viewport `set_len`/`set_content_height` plus
  `[table, running]` prior offsets follow the Package precedent; `row_rects` +
  the `Hittable` rewrite (R5/R24) are now phased deliverables.
- Phase 4 narrowed to prompt/doc cleanup on the confirm path and sharpened to
  override **both** `visibility()` and `state()` from the stored Phase 3
  fields (the `CiRunsPane` Hidden pattern).
- No findings were rejected and none required a user decision — all twelve
  were plan-completion against the settled D1–D4 / R-requirements.

### Phase 3 — Targets data model + adoption (the switch) ✅ complete (2026-06-03)

Merged from the former Phases 3 and 4 (Phase 1 review): the data model alone had
no consumer until the switch, so a standalone phase could not ship clean under
the per-phase dead-code gate.

Data model: first-seen `pid -> Instant` map and ordering on `RunningTargets`,
with `create_time` captured beside the PID (D3); member-relative display path
through `ProjectTargetSlice` (D1, R20). Add `RunningRow` and
`build_running_rows` (global, newest-at-bottom). Unit-test the ordering and the
path.

Adoption: add `Cap`. Build the Targets tree `Stack[ rows(table, Fill),
rows(running, Cap(80)) ]`. Remove the inline running display and the
`TargetDisplayKind` running variants; the table becomes one row per entry.
Render the Running box (title divider, column header, scrolled rows, newest at
bottom). Wire the highlight across both boxes. Rewire kill to the single
selected `RunningRow`'s PID — verified against `create_time` immediately before
`SIGTERM` (D3) — and remove the multi-PID path. One phase, so the Running box is
never visible but unkillable.

Carried from the Phase 1 review:

- **`Cap` sizing.** `Cap` clamps the box's **outer** height (chrome included) to
  `min(rows + chrome, floor(percent * inner))`; build it through `Size::cap`,
  which `debug_assert`s `0 < percent <= 100`. The Running box's chrome is two
  rows (divider + column header).
- **`MIN_TABLE_ROWS` floor.** Reserve the table floor before `Cap` takes its
  share so the `Fill` table cannot collapse — Phase 1's `place` lets a `Fill`
  box reach zero (`degenerate_short_terminal_zeroes_the_fill_box`).
- **`Cap` scroll arm + newest-at-bottom default.** `box_scroll_offset` matches
  only `Fixed`/`Fill` today; add a `Cap` arm reusing the keep-visible clamp
  against the capped height, and make the non-cursor default offset pin to the
  **bottom** (newest row visible, R19), not hold zero.
- **Kill is rebuilt, not just scalarized.** `resolve_kill_request` dispatches on
  `TargetDisplayKind` today; that dispatch is deleted with the table's running
  variants. Rebuild kill resolution against the highlighted `RunningRow` /
  box-index returning a single PID (R22), and sweep the `Vec<u32>` API
  (`request_kill_confirm`, `execute_target_kill`, `drop_instances`, the `pids`
  test expectations).
- **`TargetsPane` fields + remap order.** `TargetsPane` gains
  `running_cursor_pid: Option<u32>` and a per-box scroll-offset field. The
  PID→row remap (R21) runs each frame after the table row count is known, since
  the global highlight is `table_rows + running_row`.

Carried from the Phase 2 review:

- **Engine scope is exactly `Cap`.** Nesting, `Columns`, the per-stack-node
  fill invariant, the `prior_offsets` leaf order, and `.lines()` all shipped in
  Phase 2. What remains: the `Size::Cap` variant + `Size::cap` ctor, the
  `min(rows + chrome, floor(percent * inner))` clamp where `place_stack` sizes
  non-fill children, and the `Cap` arm in `box_scroll_offset` (bottom-pinned
  non-cursor default, R19).
- **No `.header()` ctor — chrome stays count-only.** The Running box reserves
  its two chrome rows (divider + column header) with count-only chrome ctors
  and the pane draws both, the way `package.rs` `render_section_rule` draws the
  Tests/crates.io titled rules. If a `.header()` name aids readability, add it
  here with its first user — it is still just a chrome-count increment.
- **The table box gets a header chrome row.** `render_targets_with_data`
  renders a ratatui `Table` with `.header(build_header_row())` and offsets the
  data area one row down by hand. Model that header as one chrome row on the
  table box so `place` accounts for it and the 80% cap / `MIN_TABLE_ROWS`
  floor measure against the true inner height; the pane keeps drawing the
  header into the box.
- **Member-dir resolution is new data-model work (D1, R20).** Nothing in the
  snapshot knows the owning member today: `detail_target_dir` is the workspace
  `target_directory`, and `classify_exe` recovers only
  `(target_dir, kind, name)` — the exe path does not encode the member.
  Resolve the owning member's manifest dir from the target name via the same
  metadata that builds `TargetEntry`/`TargetSource::Member`, and store it per
  instance/row.
- **Poller details for first-seen + `create_time` (D3, R19).** `tick()`
  rebuilds `by_key` from scratch each poll and sorts by PID; the first-seen
  `pid -> Instant` map must live outside that rebuild, and `kill`/
  `drop_instances` must drop its entries. `RunningInstance` gains
  `create_time` from sysinfo `start_time()` — extend the `ProcessRefreshKind`
  to request it.
- **Global snapshot accessor.** `build_running_rows` flattens every tracked
  key, but `RunningTargets.by_key` is private with only a per-key
  `instances(&key)` accessor — add an all-keys iterator to the snapshot.
- **Cursor-split audit beyond kill (R22).** The run path and anchors still
  read the flat table: `handle_target_action` (run/launch),
  `targets_cursor_entry` / `anchor_targets_cursor_to_entry`, and the title's
  `cursor_entry` counter. A table-region cursor maps 1:1 to an entry; a
  Running-region cursor must not trigger run/launch, and the title counter
  ignores it. Delete `display_row_for_entry`; the PID anchor (D2, R21)
  replaces the entry re-anchor in `execute_target_kill`.
- **Confirm body is rewritten once, here.** `confirm_action_body`'s multi-PID
  arm and `ConfirmAction::KillTarget { pids: Vec<u32> }` scalarize together
  with D4's `label (profile)` / `pid · started Nm ago` text, so Phase 4 keeps
  only prompt/doc-comment cleanup — no half-rewritten match arm between
  phases.
- **Viewport sync + hit-testing follow the Package precedent.** Set
  `set_len(total_selectable())` and `set_content_height` for the stacked
  two-box height; `prior_offsets` are `[table, running]` in leaf order.
  `TargetsPane` gains `row_rects: Vec<(Rect, usize)>` and its `Hittable` impl
  moves off `pos_to_local_row`, so clicks and hover resolve Running rows
  (R5, R24).

#### Retrospective

**What worked:**

- The engine addition really was exactly `Cap`: the `Size::cap` ctor, a
  `Rows::claimed_height` helper that `place_stack` uses to size every
  non-fill child (`Fixed` exact, `Cap` clamped), and the `Cap` arm in
  `box_scroll_offset` — plus four tree tests against a Targets-like fixture.
- The Package render structure transferred directly: block → `place` →
  per-box render → explicit `row_rects` → `Hittable` off the rect list.
  `targets_region` is one small function; the Running sub-pane is its own
  module (`targets/running_subpane.rs`, R10) with `RunningRow`,
  `build_running_rows`, `resolve_kill_request`, and the box renderer.
- `RunningInstance` stayed `Copy`: `first_seen: Instant` and
  `create_time: u64` are per-instance fields, while the owning member dir
  lives once per key on the snapshot (a private `RunningTarget
  { member_dir, instances }` value struct) with an `iter_targets()`
  accessor as the global all-keys view.
- `kill(pid, create_time)` refreshes the single PID, compares the live
  process's `start_time()` against the request, and skips on mismatch —
  D3's check is three lines on the existing poller.

**What deviated from the plan:**

- No `ProcessRefreshKind` extension: sysinfo collects `start_time()` with
  the base process info, so capturing `create_time` was a field read.
- The `MIN_TABLE_ROWS` floor is pane-side, not engine-side: `targets_region`
  clamps the Running box's rendered window with Phase 2's `.lines()`
  override before the engine cap applies, keeping the engine surface
  exactly `Cap`.
- The `Cap` scroll arm ignores `prior_offsets` entirely (cursor →
  keep-visible, else bottom-pin), so the pane persists no Running offset;
  `prior_offsets` is `[viewport.scroll_offset(), 0]` and the planned
  per-box-offsets field reduced to the existing viewport field alone.
- The table kept ratatui's sticky scrolling: while the highlight is in the
  table, the prior offset feeds `TableState` and ratatui adjusts it; the
  engine's table offset is used only when the highlight is in the Running
  box. Same divergence-avoidance reasoning as Phase 1's accepted cases,
  but resolved in favour of pixel-identical table behaviour.
- `running_cursor_pid` alone cannot tell "rows shifted under the cursor"
  from "the user moved the cursor", so the anchor is re-derived on every
  user move (a `navigate_targets` hook and a Targets click hook in
  `interaction.rs`) and the render-pass `sync_running_cursor` only follows
  or repairs it (gone → same-index "next" row, else previous; empty →
  last table row).
- The global list dedups multi-attributed installed binaries by PID — one
  OS process is one Running row, attribution chosen by lowest path so it
  doesn't flicker between ticks. The plan's N-way attribution was designed
  for the per-project join and said nothing about the global list.
- `Panes.detail_target_dir` and `PaneRenderCtx::running_targets_dir` were
  deleted outright: the global Running list needs no per-project join and
  the kill path no longer resolves keys, so the whole plumbing
  (`set_detail_data` param, `FinderSplit` field, detail-cache resolve) had
  no remaining reader. `render::truncate_with_suffix` went with it.
- `KillTarget`'s doc comment was rewritten here, not Phase 4 — the variant's
  fields changed (`pids: Vec<u32>` → `pid` + `create_time`), so the stale
  comment could not survive the compile anyway.
- Added `TargetsData::target_count()` so kill/anchor paths read the table
  length without cloning the entry list.

**Surprises:**

- The standing `targets_pane_row_click_selects_target` test aims clicks
  through `viewport.content_area()`; the rewrite keeps `content_area`
  pointed at the table's data rect (header excluded) so click geometry
  holds, while `set_viewport_rows` now counts both boxes' visible rows for
  page-step sizing.
- `cargo mend` rejected the `pub use RunningRow` facade (internal-only
  consumer) and the test-only `OnceLock` import landed `#[cfg(test)]`-gated
  at the top of `running_targets.rs`.

**Implications for remaining phases:**

- Phase 4's gating fact needs no new field: `running_cursor_pid.is_some()`
  is exactly "the highlight is on a Running row", and the anchor is `Some`
  only while rows exist.
- Phase 4 shrinks to the two keymap overrides plus `docs/app-api.md` sync —
  the confirm body, prompt text, and `KillTarget` doc comment all shipped
  here.

#### Phase 3 Review

- Phase 4 re-scoped to "gate `K` + wheel re-anchor": the `state()` override
  is dropped (a Hidden slot never consults it and dispatch self-guards, so
  it would be dead code), the doc-comment/prompt deliverable is marked
  shipped in Phase 3, and the `docs/app-api.md` sync is deleted — that file
  no longer exists (replaced by `docs/workspace.md`) and no surviving doc
  describes Targets/Kill/running.
- New Phase 4 deliverable: the mouse-wheel path moves the Targets highlight
  without re-deriving the PID anchor, so a wheel step inside the Running
  box snaps back to the stale anchored instance on the next render — hook
  `sync_running_cursor_pid` the way navigation and clicks already are.
- Recorded as already satisfied, so later passes don't re-open them: the
  `Shortcuts` ctx is the full `App` (the gating fact needs no plumbing),
  table-row `K` is a safe no-op today (`resolve_kill_request` returns
  `None`), and hover/click on Running rows already resolve through
  `row_rects` + global logical rows.
- Defaults and the files-touched table synced to the as-built state:
  member-relative paths (D1) replace the stale workspace-root bullet, the
  divider renders through `render_horizontal_rule` (R11), and `TargetsPane`
  carries no per-box-offset or separate gating fields —
  `viewport.scroll_offset` is the table offset and `running_cursor_pid` is
  the gate.
- No findings were rejected and none required a user decision — all nine
  were plan-consistency against the shipped Phase 3 code.

### Phase 4 — gate `K` + wheel re-anchor ✅ complete (2026-06-03)

Override `visibility()` on `TargetsPane` (`framework_keymap.rs`) so the `Kill`
hint appears only while the highlight sits on a Running row, reading
`ctx.panes.targets.running_cursor_pid().is_some()` — the anchor is `Some`
exactly when the highlight is on a Running row, so no separate gating field
exists. The change is presentational: dispatch never consults
`visibility()`/`state()`, and `handle_target_kill` already self-guards
(`resolve_kill_request` returns `None` on table rows), so a table-row `K` is
a safe no-op today.

Route the Targets mouse-wheel scroll through the PID re-anchor: the wheel
path (`input/mod.rs` → `viewport_mut_for(...).up()/down()`) moves the
highlight without calling `sync_running_cursor_pid`, so a wheel step within
the Running box snaps back to the stale anchored instance on the next
render. Hook it the way navigation (`navigate_targets`) and clicks
(`interaction.rs`) already are.

Resolved against the as-built code (Phase 3 review):

- **`state()` override dropped.** The `CiRunsPane` pattern overrides only
  `visibility()`; a Hidden slot is dropped from the bar before `state()` is
  ever consulted, and there is no Visible-but-Disabled case for `Kill` — a
  `state()` override would be dead code under the per-phase gate.
- **Ctx reachability is a non-issue.** `render_bar_slots` passes the full
  `App` as the `Shortcuts` ctx, so the override reads
  `running_cursor_pid()` directly; the Phase 1 "persist the gating fact"
  bullet is satisfied by that field alone.
- **No doc sync remains.** `docs/app-api.md` was deleted (replaced by
  `docs/workspace.md`) and no surviving doc describes Targets/Kill/running;
  the `KillTarget` doc comment, confirm body, and prompt text all shipped
  with Phase 3's field changes.
- **Hover on Running rows already works.** `render_rows` styles by global
  logical row through `selection_state`, and `TargetsPane::hit_test_at`
  resolves Running rows from `row_rects` — no Phase 4 work.

#### Retrospective

**What worked:**

- The `CiRunsPane` pattern transplanted directly: a `visibility()` match
  with a catch-all `Visible` arm plus a `targets_kill_visibility` helper
  reading `running_cursor_pid().is_some()` (`framework_keymap.rs`).
- The wheel hook is three lines in `scroll_pane_at` (`input/mod.rs`): after
  the viewport step, `pane_id == PaneId::Targets` calls
  `panes::sync_running_targets_cursor`, mirroring the click hook in
  `interaction.rs`.

**What deviated from the plan:**

- The bar-snapshot test
  (`focused_app_panes_render_expected_pane_action_labels`) hardcoded `kill`
  in the default Targets bar and failed once the gate landed. It split into
  an un-anchored case (`run`/`release` only) and an anchored case
  (`set_running_cursor_pid(Some(..))` shows `kill`), plus a direct
  visibility test pair mirroring the CiRuns ones.

**Surprises:**

- `Option::is_some` is const-eligible, so clippy's `missing_const_for_fn`
  required `const fn` on the visibility helper — unlike the CiRuns helpers,
  which call non-const accessors.

**Implications for remaining phases:**

- None — this was the final phase; the plan is complete.

#### Phase 4 Review

Closure pass over the whole plan; every finding was minor and none required
a user decision.

- Every promised behavior verified against shipped code: single-PID kill
  with create-time validation, continuous cursor with newest-at-bottom
  ordering, `Cap(80)` + `MIN_TABLE_ROWS` floor, present-only-when-running,
  member-relative paths (D1), and the `K` gate.
- The hidden `K` slot is confirmed purely presentational:
  `resolve_kill_request` returns `None` on every table row, so a bound but
  hidden `K` stays a no-op.
- All cursor-moving paths audited: Home/End/Page/HalfPage funnel through
  `navigate_targets`'s single sync call; `reset_project_panes` (project
  switch) and finder `navigate_to_target` land on table rows, where the
  render-pass `sync_running_cursor` clears the anchor — no hooks needed.
- Invariant for future input paths: the render-pass reconcile self-heals
  every case except a move *within* the Running box while a live anchored
  PID is set — any new path that can do that needs the
  `sync_running_targets_cursor` hook (wheel was exactly this case).
- §`K` hint gating prose synced to the as-built single `visibility()`
  override (the `state()` half was dropped in the Phase 3 review).
- R1's "Targets holds two offsets" wording is superseded by the as-built
  single-offset design but stays as a historical cycle log; the live
  Defaults section already records the shipped reality.

## Defaults taken (call out to change)

- The sub-pane title shows a count: `Running (3)`, or `Running (2 of 3)`
  while the highlight is in the box.
- The divider's `├ ┤` tees merge into the side borders through
  `tui_pane::render_horizontal_rule` spanning the full pane width (R11).
- The table reserves a footer boundary row above the Running divider
  (`Region::footer()`, engine-owned): blank while every table row is
  visible, a full-width rule with the pager centered on it (the shared
  `render_overflow_affordance`, `label_color`) once it scrolls — the pager
  never overlaps the last table row.
- Installed (`cargo`-profile) instances fold under a collapsible
  `▶ cargo (N)` header row at the top of the Running list, collapsed by
  default (`CargoGroup` on `TargetsPane`; some are always running —
  cargo-port itself at minimum). `Right`/`Left` (and vim `l`/`h`, via the
  shared navigation scope `navigate_targets` intercepts) expand/collapse
  with the project list's row idiom — fall through to a row move when not
  on the group — and `Enter` on the header toggles; the header has no
  PID, so `K` hides there and the kill resolver skips it; the divider's
  `(i of N)` counts instances, with collapsed cargo rows occupying
  `1..=count`.
- The Running list is global, so it renders — and the pane stays
  tab-reachable — even when the selected project has no targets
  (`has_instances` joins `has_targets` in both gates); the
  `MIN_TABLE_ROWS` floor drops when the table is empty. The empty block
  shows only when there are no targets and nothing running.
- `TargetsPane` keeps its `Viewport` (the highlighted row, whose
  `scroll_offset` doubles as the table box's persisted offset) and gains
  `running_cursor_pid` (also the `K`-gating fact) and hit-test `row_rects` —
  no rename of the existing field, and no Running offset field since the
  `Cap` box derives its offset every frame.
- Workspace-member running targets display the member-relative path
  (per D1) — each row resolves its owning member's manifest dir, falling
  back to the workspace root for stale artifacts.

## Review refinements (cycle 1, auto-recorded)

Resolved by the team review; one sensible in-intent outcome each. Accepted.

> **Fewest-types directive (overrides parts of the review).** The model uses the
> smallest set of types that mirrors the display: `Region`, `Size`, the `Placed`
> layout result, and the existing `Viewport` for the highlight. This drops the
> review's newtype-heavy suggestions — no `SegmentIndex`, no `CursorLocation`, no
> validated-fraction newtype, no sorted-rows wrapper — in favour of plain values
> and reuse. R1–R4 and R9 are restated to match.

- **R1 — Per-box scroll offsets.** Each scrolling box keeps its own offset; the
  pane holds these across frames and hands them to the layout each frame (the CPU
  pane keeps one, Targets holds two). The layout re-clamps each on resize via
  `keep_visible_scroll_offset`.
- **R2 — Sizes.** `Fixed` takes exactly the box's rows; `Cap(percent)` takes
  `min(rows + chrome, floor(percent * parent_height))`; the one `Fill` per stack
  takes the remainder. Because Running is `Cap(80)`, the table keeps ≥20% and
  cannot vanish.
- **R3 — One `Fill` per stack.** A stack has exactly one `Fill` box; the layout
  treats that as an invariant. `Cap` takes a plain percent number — no newtype.
- **R4 — Which box has the highlight.** The layout answers this directly (no
  index newtypes); render, kill, and hint-gating read it from the layout result.
- **R5 — Mouse + hover are box-aware.** Targets adopts the CPU pane's explicit
  `row_rects` hit-testing (record `(rect, logical_row)` per rendered row in both
  boxes) instead of the flat `pos_to_local_row`; hover styling reads which box the
  row is in; clicking a Running row selects it.
- **R6 — Navigation (one highlight over `0..len`).** Down/Up cross the box
  boundary; `Home` → row 0 (table top); `End` → last Running row; `Page` /
  `HalfPage` step within the unified range. Each scrolling box holds its offset
  while the highlight is in another box, so rows don't jump on entry.
- **R7 — Single-PID `KillRequest`.** `KillRequest` carries `pid: u32`, not
  `Vec<u32>`. `request_kill_confirm` / `execute_target_kill` /
  `resolve_kill_request` and the confirm body become scalar; the
  `ConfirmAction::KillTarget` doc drops the multi-instance wording.
  `resolve_kill_request` takes the typed `CursorLocation` + running rows and
  returns `Some` only in the Running segment.
- **R8 — First-seen map lifecycle.** Persist `pid -> Instant`; after each tick
  retain only PIDs present in the snapshot, and drop on kill. Optional hardening:
  pair with the process `create_time` (sysinfo) to disambiguate PID reuse;
  without it, ordering is stable within a frame and reshuffles only on rapid
  reuse.
- **R9 — `RunningRow` types.** `display_path` uses the existing `DisplayPath`,
  not `String`; `profile` reuses `RunProfile`. `build_running_rows` sorts
  newest-at-bottom once before returning — a plain sorted `Vec`, no wrapper type.
- **R10 — Module naming.** Name the new view module to avoid collision with
  `running_targets.rs` — e.g. `targets/running_subpane.rs`.
  `render_targets_pane_body` calls `build_running_rows` + `render_running_subpane`
  directly; `build_target_display_rows` becomes table-only (one row per entry) or
  is deleted.
- **R11 — Divider rendering.** Render `├─ Running (N) ─┤` through a `tui_pane`
  helper (extend `render_rules` / add `render_sub_pane_divider`) with a width
  guard (skip below 3 columns), not ad-hoc buffer writes. Supersedes the earlier
  "direct buffer-cell writes" default.
- **R12 — Narrow-pane path truncation.** Left-truncate the Path column (show the
  tail, e.g. `…/bevy_window_manager`) so the project name stays visible; reuse the
  existing ellipsis helper.
- **R13 — Title consistency.** Build the sub-pane title with the existing
  `PaneTitleCount` machinery so it reads `Running (N)` and shows `(N of M)`
  pagination when the list scrolls, consistent with the CPU pane.
- **R14 — Layout test gate.** Before Phase 1 ships, unit-test the tree: box
  sizing (`Fixed` / `Fill`), which-box-has-the-highlight at box boundaries, and
  per-box offset persistence across highlight moves and resize. The CPU pane's
  existing tests guard the rebuild.
- **R15 — Phase hygiene.** The data-model items must compile clean under denied
  unused/dead-code lints. Resolved by the Phase 1 review: the data model is
  merged into Phase 3 (data model + adoption) so it lands with its consumer.
  `build_running_rows` is `O(N log N)` per frame — fine for realistic counts;
  cache if a perf log shows it over 1ms.

## Review refinements (cycle 2, auto-recorded)

Second team pass. Each has one sensible in-intent outcome (spec detail, test,
constant, or correctness fix); accepted and folded into the phases above. The two
genuine product/safety choices the team surfaced are D3 and D4 below.

- **R16 — Write the layout contract before Phase 1.** The layout pass is
  specified, not left implicit: (a) sizing order is `Fixed` exact, then each
  `Cap` clamped to `min(rows + chrome, floor(percent * inner_height))`, then the
  one `Fill` takes the remainder; (b) "exactly one `Fill` per `Stack`" is a
  `debug_assert!` at layout entry — no newtype (fewest-types); (c) `Cap`'s percent
  is a plain number, built through a `Size::cap(percent)` constructor that
  `debug_assert!`s `0 < percent <= 100`; (d) a box's **chrome rows** (title
  divider, column header) count toward its outer height, so `Cap` is applied to
  the outer height including chrome; (e) the single `Fill` box keeps at least a
  floor of rows (`MIN_TABLE_ROWS`, R23) so a short terminal can't zero the table;
  (f) the layout result answers "which box holds the highlight, and the row within
  it" over plain index ranges — no `CursorLocation` newtype; (g) `Viewport::len`
  is the sum of selectable rows across all boxes, set once per frame — documented
  on `set_len`.
- **R17 — Layout test gate (extends R14).** Phase 1 unit-tests: `Fixed`/`Fill`/
  `Cap` sizing; which-box-has-the-highlight at box boundaries; per-box offset
  persistence across highlight moves and on resize; the `Cap` floor keeps the
  table `>= MIN_TABLE_ROWS`; the degenerate short-terminal case; and a
  layout-construction timing assertion (rebuild many times, well under the frame
  budget) so per-frame tree rebuild cost is guarded, not assumed.
- **R18 — Guard the `Band` deletion by porting its tests.** `cpu_band_offset`'s
  tests and `tests_band_offset_for`'s tests are rewritten to assert the tree's
  offsets *before* `Band` is deleted. `Band` is removed only once both the CPU
  pane (Phase 1) and the detail pane (Phase 2) run on the tree and their ported
  tests pass. Phase 2 acceptance: the detail pane behaves identically — Structure
  pinned, Tests scroll, crates.io pinned, cross-column cursor, and each column's
  offset persists while the highlight is in the other.
- **R19 — First-seen map lives on the poller.** `pid -> Instant` is a field on the
  poller that persists across ticks (`RunningTargetsPoller`), not on the
  per-frame snapshot. In `tick(now)`: insert unseen PIDs with `now`; after
  rebuilding the snapshot, retain only PIDs still present; drop on kill.
  `build_running_rows` sorts by first-seen ascending, so a new instance appends at
  the **bottom** of the list (it is not inserted at the top — the box's top edge
  rises as the list grows). Unit-test insert / retain / drop and the ordering.
- **R20 — Thread the member directory through the snapshot.** `ProjectTargetSlice`
  (`src/tui/running_targets.rs`) carries only `target_dir`; add the owning member
  directory (the package manifest dir) per running target so the Path column is
  member-relative (D1). A bench whose exe is `target/<profile>/deps/<name>-<hash>`
  resolves its member from the bench name via metadata; if that can't be resolved,
  the path falls back to the workspace root and the row notes it. This lands in
  Phase 3 (data model + adoption) alongside its consumer.
- **R21 — PID-anchor cursor mechanics (implements D2).** `TargetsPane` gains a
  plain `running_cursor_pid: Option<u32>` (no newtype). When the highlight enters
  the Running box, record the row's PID; each frame after `build_running_rows`,
  map that PID back to its current row and move the highlight there; clear it when
  the highlight leaves the box. Clamp on loss: when the anchored PID is gone, move
  to the adjacent Running row (next, else previous); when the box empties, fall
  back into the table, restoring the last table highlight position (clamped). When
  the sub-pane appears under the user (a process starts), the cursor stays where
  it is — it does not jump into Running.
- **R22 — `KillRequest` scalarization is an adoption-phase (Phase 3) audit.** `KillRequest` becomes
  `{ label, pid: u32 }` (see D3 for the create-time field). Grep every
  construction and `pids` read site; scalarize `request_kill_confirm`,
  `execute_target_kill`, `resolve_kill_request`, and the confirm body;
  `resolve_kill_request` returns `Some` only when the highlight is in the Running
  box. Update the test expectations that assert a `pids` vec.
- **R23 — Name the constants.** `constants.rs` (or a Targets-local module) names:
  the cap percent (`80`), the sub-pane chrome rows (divider + header), the minimum
  table rows, and the column widths. No bare numbers in the layout or render path.
- **R24 — Box-aware hit-testing and per-box title count.** `row_rects` records the
  box per rendered row so hover, click-to-select, and styling know which box a row
  is in. The Running title uses `PaneTitleCount::Single { len: running_len, cursor:
  running-local }` so it reads `Running (N of M)` when scrolled; the table keeps
  its grouped count. The table and Running scroll offsets each persist across
  highlight moves (R1).
- **R25 — Path truncation keeps the member segment (refines R12).** The Path
  column's left-truncation keeps the rightmost member segment visible (e.g.
  `…/bevy_window_manager`); a generic right-trim that could drop the member name
  is not used.
- **R26 — The Running list is global, by intent.** It lists every tracked
  instance across all sections, on every section. The Target column disambiguates,
  so a row may reference a target not shown in the current section's table; this is
  expected and needs no section filter (filtering would regress the stated intent).

## Proposed user decisions

- **D1 — Same-named targets across workspace members** (severity: important;
  source: User Impact & Ergonomics; class: design-improvement; status: decided —
  member-relative path).
  The recorded default shows the workspace-root path (`~/workspace`) for every
  member and omits the package/member name. Two members that export the same
  binary name (both have `my_app`) then render identical Target + Path, so only
  the PID distinguishes them — which defeats "so we know which one." Options:
  (a) add a Package/Member column; (b) show the member-relative path instead of
  the workspace root; (c) accept PID-only disambiguation. Recommendation: (a) add
  the package/member identifier — the smallest reliable disambiguator — accepting
  the tighter width (see R12).
  **Decision:** show the member-relative path (option b) — e.g.
  `~/rust/bevy_window_manager/foo` — instead of the workspace root. The Path
  column resolves to each running instance's own member directory; left
  truncation (R12) keeps the member name visible in a narrow pane.
- **D2 — Cursor anchoring as the running list changes** (severity: important;
  source: User Impact & Ergonomics + Risk; class: design-improvement; status:
  decided — anchor by PID). With newest-at-bottom and grow-upward, an instance starting or
  exiting while the cursor is in the Running list leaves a fixed logical index
  pointing at a *different* instance — and since `K` kills the row under the
  cursor, a row shifting under it risks killing the wrong process. Options:
  (a) anchor the cursor to the selected instance by PID across ticks (it follows
  its instance as rows reorder/grow); (b) keep a fixed row index (simpler, but the
  selection can slide to another instance). Recommendation: (a), mirroring the
  existing post-kill entry-anchoring. This keeps the chosen newest-at-bottom /
  grow-upward behaviour and only stabilises *which* instance is selected.
  **Decision:** anchor the cursor to the selected instance by PID while it is in
  the Running segment; it follows its instance as rows reorder or grow. When the
  anchored instance is killed or exits, fall to the adjacent Running row, then
  back into the table once the list empties.
- **D3 — Kill safety against PID reuse** (severity: critical; source: Risk &
  Failure Modes + Type System; class: design-improvement; status: decided —
  require create-time validation).
  `K` kills the PID under the cursor. The PID is read from one snapshot when the
  confirm dialog opens and the kill fires from a later snapshot after the user
  presses `Y`. If the target exits in between and the OS reassigns that PID number
  to an unrelated process, `SIGTERM` lands on the wrong process. R8 currently
  files create-time pairing as *optional* hardening. Three of the five lenses
  flagged this as the most dangerous failure mode of per-instance kill. Options:
  (a) require it now — carry `create_time` (sysinfo) alongside the PID in the
  first-seen map and the `KillRequest`, and verify both PID and create-time match
  the live process immediately before `SIGTERM`, skipping the kill on mismatch;
  (b) keep it optional — accept that rapid PID reuse can mis-target, ordering is
  otherwise stable within a frame. Recommendation: (a) — it is the difference
  between "kills the row you see" and "usually kills the row you see," for one
  extra field read per process per tick.
  **Decision:** require it now (option a). Carry `create_time` (sysinfo) beside
  the PID in the first-seen map and in `KillRequest`; before `SIGTERM`, verify
  both the PID and its create-time against the live process and skip the kill on
  mismatch. This makes create-time a required field of the kill path, not the
  optional hardening R8 described — R8 is updated to match.
- **D4 — Kill confirm dialog instance identity** (severity: important; source:
  User Impact & Ergonomics; class: design-improvement; status: decided — label +
  profile + PID + start-age). With
  per-instance kill, two instances of the same target (same name, different
  profile or member path) are distinct Running rows. The confirm body currently
  shows the target label and the PID. For same-named instances that reads
  `my_app` / `pid 48213` — the user can't tell which profile or member is about to
  die without trusting the PID alone. `RunningRow` already carries `profile` and
  `display_path`. Options: (a) label + PID only (status quo); (b) label + profile
  + PID; (c) label + profile + member path + PID. Recommendation: (b) — profile is
  the cheapest disambiguator that covers the common same-target/different-profile
  case; add the path only if same-profile/different-member instances are a real
  scenario.
  **Decision:** label + profile + PID + start-age (option d). The confirm body
  reads e.g. `my_app (debug)` / `pid 48213 · started 2m ago`. Start-age is derived
  from the `create_time` D3 already captures, so two identical instances (same
  target, profile, and member) are still distinguishable in the dialog by age, not
  only by a bare PID. The member path is not shown in the body; the Running row
  the cursor selected already shows it.
