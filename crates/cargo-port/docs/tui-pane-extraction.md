# tui_pane extraction plan

The initial `tui_pane` extraction left generic UI primitives in `src/tui/`.
This plan moves them across the workspace boundary so app-specific code
lives in `src/` and reusable framework code lives in `tui_pane/`.

The mouse hit-test dispatch and the per-pane render dispatch are the
two headline payloads. They move in separate phases because the render
dispatch requires a cargo-port-side prerequisite (every pane must
funnel through one trait) that the hit-test dispatch does not need.

## Phase overview

| Phase | What | Generics work | Risk | Rough size |
|-------|------|---------------|------|------------|
| 1 | Leaf utilities + color palette | None | Low | ~5 files moved, one commit |
| 2 | Pane chrome bundle + layout state moves onto Framework | None | Low | ~7 files moved, `LayoutCache` dissolved, one commit |
| 3 | Generic `Hittable` trait + hit-test dispatch loop | Associated `Target` type | Medium | 1 trait moved, 11 impl sites updated, one commit |
| 4 | Cargo-port-side render unification — tier-one panes | None | Medium | Targets/Lints/CiRuns/Output routed through `Pane::render`, `PaneRenderCtx` widened with `inflight`, one commit |
| 5 | Cargo-port-side render unification — tier-two panes | None | Medium | `ProjectList` body and 3 overlay popups absorbed into `impl Pane`, `PaneRenderCtx` further widened (scan/ci/lint/inline_error), one commit |
| 6 | `Renderable` trait + render dispatch loop | Generic `Ctx` param + HRTB on registry, new `PaneRegistry` trait | Medium | 1 trait moved, 11 impl sites renamed, render orchestration switches to generic loop, one commit |

Each phase lands as a single commit after `cargo build && cargo nextest
run && cargo clippy && cargo mend && cargo +nightly fmt` all pass green.

## Target layout (post-extraction)

```
tui_pane/src/
├── activity.rs               (unchanged)
├── app_context.rs            (unchanged)
├── bar/                      (unchanged)
├── constants.rs              ← generic palette extracted from src/tui/constants.rs
├── dispatch/
│   ├── mod.rs                ← Hittable trait (Phase 3) + Renderable trait + PaneRegistry (Phase 6)
│   ├── hit_test.rs           ← generic hit-test dispatch loop (Phase 3)
│   └── render.rs             ← generic render dispatch loop (Phase 6)
├── framework/                (unchanged)
├── keymap/                   (unchanged)
├── layout.rs                 ← was src/tui/pane/layout.rs
├── lib.rs                    (expanded re-exports)
├── pane.rs                   (unchanged — Pane<Ctx> identity trait)
├── pane_chrome.rs            ← was src/tui/pane/chrome.rs
├── pane_id.rs                (unchanged)
├── pane_state.rs             ← was src/tui/pane/state.rs
├── pane_title.rs             ← was src/tui/pane/title.rs
├── panes/                    (unchanged)
├── popup.rs                  ← was src/tui/overlays/popup.rs
├── rules.rs                  ← was src/tui/pane/rules.rs
├── running_tracker.rs        ← was src/tui/support/running_tracker.rs
├── settings_store/           (unchanged)
├── table/
│   ├── mod.rs
│   └── widths.rs             ← was src/tui/columns/widths.rs
├── toasts/                   (unchanged)
├── util.rs                   ← `format_progressive` (room for future free fns)
├── viewport.rs               (unchanged)
└── watched_file.rs           ← was src/tui/support/watched_file.rs
```

## Phase 1 — Leaf utilities and color palette

Pure-additive moves. Every file in this phase has zero references to
cargo-port domain types and can be relocated as-is. The only seam work is
trimming a couple of consumer-specific comments.

### What moves

- `src/tui/support/duration.rs::format_progressive` → `tui_pane/src/util.rs`
  - `util.rs` is the catch-all for free functions tui_pane exports
    that don't have a natural home. Convention: `support/` is for
    internal helpers; `util/` (or a `util.rs` file) is for public
    free functions.
- `src/tui/support/watched_file.rs` → `tui_pane/src/watched_file.rs`
  - `WatchedFile<T>` — concept-bearing type, stands on its own at
    crate root next to `Viewport`, `Activity`, etc.
- `src/tui/support/running_tracker.rs` → `tui_pane/src/running_tracker.rs`
  - `RunningTracker<K>` (after scrubbing lint/GitHub references from
    comments). Same rationale — concept-bearing type, crate-root.
- Generic color palette from `src/tui/constants.rs` → `tui_pane/src/constants.rs`
  - `ACCENT_COLOR`, `ACTIVE_BORDER_COLOR`, `ACTIVE_FOCUS_COLOR`, `HOVER_FOCUS_COLOR`,
    `COLUMN_HEADER_COLOR`, `DISCOVERY_SHIMMER_COLOR`, `ERROR_COLOR`,
    `INLINE_ERROR_COLOR`, `INACTIVE_BORDER_COLOR`, `INACTIVE_TITLE_COLOR`,
    `LABEL_COLOR`, `REMEMBERED_FOCUS_COLOR`, `SECONDARY_TEXT_COLOR`,
    `STATUS_BAR_COLOR`, `SUCCESS_COLOR`, `TARGET_BENCH_COLOR`, `TITLE_COLOR`,
    `FINDER_MATCH_BG`
  - Generic dimensions: `BLOCK_BORDER_WIDTH`, `BYTES_PER_KIB`, `BYTES_PER_MIB`,
    `BYTES_PER_GIB`, `FRAME_POLL_MILLIS`, `SECTION_HEADER_INDENT`,
    `SECTION_ITEM_INDENT`

### What stays in `src/tui/constants.rs`

- `FINDER_POPUP_HEIGHT`, `SETTINGS_POPUP_WIDTH`, `CONFIRM_DIALOG_HEIGHT`,
  `CI_TIMESTAMP_WIDTH`, `MAX_FINDER_RESULTS` — app-specific popup dims
- `STARTUP_PHASE_DISK`, `STARTUP_PHASE_GIT`, `STARTUP_PHASE_GITHUB`,
  `STARTUP_PHASE_LINT`, `STARTUP_PHASE_METADATA` — app startup labels

### Module re-exports

`tui_pane/src/lib.rs` adds:
```rust
mod constants;
mod running_tracker;
mod util;
mod watched_file;

pub use constants::{
    ACCENT_COLOR, ACTIVE_BORDER_COLOR, ACTIVE_FOCUS_COLOR, BLOCK_BORDER_WIDTH,
    BYTES_PER_GIB, BYTES_PER_KIB, BYTES_PER_MIB, COLUMN_HEADER_COLOR,
    DISCOVERY_SHIMMER_COLOR, ERROR_COLOR, FINDER_MATCH_BG, FRAME_POLL_MILLIS,
    HOVER_FOCUS_COLOR, INACTIVE_BORDER_COLOR, INACTIVE_TITLE_COLOR,
    INLINE_ERROR_COLOR, LABEL_COLOR, REMEMBERED_FOCUS_COLOR,
    SECONDARY_TEXT_COLOR, SECTION_HEADER_INDENT, SECTION_ITEM_INDENT,
    STATUS_BAR_COLOR, SUCCESS_COLOR, TARGET_BENCH_COLOR, TITLE_COLOR,
};
pub use running_tracker::RunningTracker;
pub use util::format_progressive;
pub use watched_file::WatchedFile;
```

### Sequencing

1. Create `tui_pane/src/{util,watched_file,running_tracker}.rs` with content
   moved verbatim. Strip the lint/GitHub references from
   `running_tracker.rs` comments.
2. Create `tui_pane/src/constants.rs` with the generic palette and dimensions.
3. Update `tui_pane/src/lib.rs` with the `mod` declarations and re-exports above.
4. In `src/`, replace every `crate::tui::support::duration::*`,
   `crate::tui::support::watched_file::*`, `crate::tui::support::running_tracker::*`
   with `tui_pane::*`. Replace every moved constant import with `tui_pane::*`.
5. Delete `src/tui/support/duration.rs`, `src/tui/support/watched_file.rs`,
   `src/tui/support/running_tracker.rs`. Trim the moved constants out of
   `src/tui/constants.rs`. Update `src/tui/support/mod.rs` to drop empty mods.
6. Pre-flight: confirm no new `tui_pane` re-export name collides with
   an identifier used by the `action_enum!` or `bindings!` macros
   (defined in `tui_pane/src/keymap/action_enum.rs` and
   `tui_pane/src/keymap/bindings.rs`; both use `$crate::*` paths).
   Cross-check `rg 'pub use' tui_pane/src/lib.rs` against
   `rg '\$crate::\w+' tui_pane/src/keymap/action_enum.rs tui_pane/src/keymap/bindings.rs`.
   Current additions (`WatchedFile`, `RunningTracker`,
   `format_progressive`, color/dim consts) are unlikely to collide,
   but verify once.
7. Cargo checkpoint.

### End-state contents

After Phase 1:
- `src/tui/support/mod.rs` — drops the three `pub mod` declarations; remaining
  files (if any) keep their declarations.
- `src/tui/constants.rs` — keeps only the app-specific consts listed under
  "What stays in `src/tui/constants.rs`" above.

## Phase 2 — Pane chrome bundle

All files in this phase render UI primitives that depend only on ratatui
and the palette moved in Phase 1. No generics work needed — these traits
and structs do not reference cargo-port types.

### What moves

- `src/tui/pane/chrome.rs` → `tui_pane/src/pane_chrome.rs`
  - `PaneChrome`, `default_pane_chrome`, `empty_pane_block`
- `src/tui/pane/title.rs` → `tui_pane/src/pane_title.rs`
  - title formatting helpers with count + cursor
- `src/tui/pane/state.rs` → `tui_pane/src/pane_state.rs`
  - `PaneFocusState`, `PaneSelectionState`, `scroll_indicator`
- `src/tui/pane/rules.rs` → `tui_pane/src/rules.rs`
  - horizontal/vertical rule widget with titles and connectors
- `src/tui/pane/layout.rs` → `tui_pane/src/layout.rs`
  - `PaneGridLayout`, `PanePlacement`, `PaneAxisSize`, `PaneSizeSpec`,
    `ResolvedPane`, `ResolvedPaneLayout`

### Layout state moves onto `Framework`

`App.layout_cache` (currently a `LayoutCache` struct at
`src/tui/panes/layout.rs:17` holding `tiled: ResolvedPaneLayout<PaneId>`
and `project_list_body: Rect`) is framework-flavored state stored on
the app — render writes it, input reads it. After this phase:

- The `tiled` field moves onto `tui_pane::Framework` as
  `tiled_layout: Option<ResolvedPaneLayout<AppPaneId>>`, with accessors
  `framework.tiled_layout()` (read) and `framework.set_tiled_layout(...)`
  (write). Render computes and stores; input reads.
- The `project_list_body` field moves onto `ProjectListPane` as
  `body_rect: Rect`. The pane records it during its own render; input
  reads it via the pane registry or a direct accessor.
- The `LayoutCache` struct itself is deleted. `App.layout_cache` field
  is deleted.
- All call sites in `src/tui/input/mod.rs` (lines 429, 439, 534, 536)
  and `src/tui/panes/project_list.rs` (lines 152, 169, 203) repath to
  the new owners.
- `src/tui/columns/widths.rs` → `tui_pane/src/table/widths.rs`
  - `ColumnWidths`, `ColumnSpec`
- `src/tui/overlays/popup.rs` → `tui_pane/src/popup.rs`
  - `PopupFrame`

### What stays

- `src/tui/columns/mod.rs` — 1177 lines of column-definition code stays
  put. After `widths.rs` moves out, `columns/` is a single-file directory
  (`mod.rs` only). Can be flattened to `src/tui/columns.rs` if desired,
  but not required.
- `src/tui/panes/widths.rs` (260 lines, `ProjectListWidths`) — wraps the
  generic `ColumnWidths` with project-list-specific column metadata.
  Stays in cargo-port; switches its import from
  `crate::tui::columns::widths::ColumnWidths` to `tui_pane::ColumnWidths`.
- `src/tui/overlays/` — keeps `pane_impls.rs` (app-side overlay panes:
  `KeymapPane`, `SettingsPane`, `FinderPane` impls), `render_state.rs`
  (the `FinderPane` viewport-wrapper struct — app-owned, per its own
  doc header), and `mod.rs`.
- `src/tui/pane/` — keeps `dismiss.rs` and `dispatch.rs` until Phase 3.
  The `mod.rs` shrinks; do not delete the directory yet.

### Module re-exports

`tui_pane/src/lib.rs` adds:
```rust
mod layout;
mod pane_chrome;
mod pane_state;
mod pane_title;
mod popup;
mod rules;
mod table;

pub use layout::{PaneAxisSize, PaneGridLayout, PanePlacement, PaneSizeSpec};
pub use pane_chrome::{PaneChrome, default_pane_chrome, empty_pane_block};
pub use pane_state::{PaneFocusState, PaneSelectionState, scroll_indicator};
pub use pane_title::{/* title fns */};
pub use popup::PopupFrame;
pub use rules::{/* rule fns */};
pub use table::{ColumnSpec, ColumnWidths};
```

### Sequencing

1. Create the new tui_pane files at crate root:
   `tui_pane/src/{pane_chrome,pane_title,pane_state,layout,popup,rules}.rs`
   and the `tui_pane/src/table/{mod,widths}.rs` tree. Move content
   verbatim — references to `ACTIVE_BORDER_COLOR` etc. now resolve
   inside `tui_pane` against Phase 1's `crate::constants`.
2. Update `tui_pane/src/lib.rs` with the new `mod` declarations + re-exports.
3. In `src/`, repath every consumer:
   - `crate::tui::pane::chrome::PaneChrome` → `tui_pane::PaneChrome`
   - `crate::tui::pane::title::*` → `tui_pane::*`
   - `crate::tui::pane::state::PaneFocusState` → `tui_pane::PaneFocusState`
   - `crate::tui::pane::rules::*` → `tui_pane::*`
   - `crate::tui::pane::layout::*` → `tui_pane::*`
   - `crate::tui::columns::widths::*` → `tui_pane::*` (re-exported from `tui_pane::table`)
   - `crate::tui::overlays::popup::PopupFrame` → `tui_pane::PopupFrame`
4. Add `tiled_layout: Option<ResolvedPaneLayout<AppPaneId>>` field to
   `tui_pane::Framework` along with `tiled_layout()` /
   `set_tiled_layout(...)` accessors. Add `body_rect: Rect` field to
   cargo-port's `ProjectListPane` struct. Delete the `LayoutCache`
   struct and `App.layout_cache` field. Repath the 6 call sites
   listed in "Layout state moves onto Framework" above.
5. Delete the moved files. Trim `src/tui/pane/mod.rs`,
   `src/tui/columns/mod.rs`, `src/tui/overlays/mod.rs` accordingly.
6. **⚠️ Import-path trap.** Update test imports in `src/tui/pane/state.rs`'s
   test module: change `use super::ACTIVE_FOCUS_COLOR;` (and
   `REMEMBERED_FOCUS_COLOR`, `HOVER_FOCUS_COLOR`) to
   `use crate::constants::{ACTIVE_FOCUS_COLOR, REMEMBERED_FOCUS_COLOR,
   HOVER_FOCUS_COLOR};`. After the file moves to
   `tui_pane/src/pane_state.rs`, `super::` resolves to `pane_state`,
   not to the constants — the explicit `crate::constants::*` path is
   required. This is a silent rewrite (no compile error if missed
   because the test wouldn't compile anyway). Confirm with
   `cargo nextest run -p tui_pane` before the checkpoint.
7. Cargo checkpoint.

### End-state contents

After Phase 2, `src/tui/pane/` contains:
- `mod.rs` — declares `pub mod dismiss; pub mod dispatch;` plus
  re-exports of the moved tui_pane items if call-site convenience is
  preferred (e.g. `pub use tui_pane::{PaneChrome, PaneFocusState};`).
- `dispatch.rs` — unchanged (Phase 3 handles it).
- `dismiss.rs` — unchanged (kept in cargo-port).

`src/tui/columns/` is gone or collapsed to one file with no contents.
`src/tui/overlays/` keeps `mod.rs` and `pane_impls.rs`.

## Phase 3 — Generic `Hittable` trait and hit-test dispatch loop

Moves the reusable hit-test machinery: the `Hittable` trait, the
z-order walk, and the first-hit-wins dispatch helper. The render trait
(`Pane` in `src/tui/pane/dispatch.rs`) stays in cargo-port for now —
it can't move until every cargo-port pane funnels through one render
trait — that's Phases 4 and 5's job. The trait move itself happens in Phase 6.

### Trait design

The current cargo-port `Hittable`:
```rust
pub trait Hittable: Pane {
    fn hit_test_at(&self, pos: Position) -> Option<HoverTarget>;
}
```

Becomes (in `tui_pane`):
```rust
pub trait Hittable {
    type Target;
    fn hit_test_at(&self, pos: Position) -> Option<Self::Target>;
}
```

Two design decisions:

1. **No `Ctx` associated type.** `hit_test_at` doesn't consume a
   context — it only takes the cursor position. Adding `type Ctx<'a>`
   as a GAT would make the trait non-object-safe (Rust's rule: traits
   with generic associated types cannot be used as `&dyn Trait`),
   blocking the dispatch loop. The trait stays clean: one associated
   type (`Target`), one method.
2. **No `Hittable: Renderable` supertrait.** Render and hit-test are
   orthogonal concerns; coupling them blocks future panes that
   participate in one but not the other. Each pane impls them
   independently.

### Dispatch loop

The generic hit-test dispatch lives in `tui_pane/src/dispatch/hit_test.rs`:

```rust
pub trait HitTestRegistry {
    type PaneId: Copy;
    type Target;
    fn z_order() -> &'static [Self::PaneId];
    fn pane(&self, id: Self::PaneId) -> Option<&dyn Hittable<Target = Self::Target>>;
}

pub fn hit_test_at<R: HitTestRegistry>(registry: &R, pos: Position) -> Option<R::Target> {
    for id in R::z_order() {
        if let Some(pane) = registry.pane(*id)
            && let Some(hit) = pane.hit_test_at(pos) {
            return Some(hit);
        }
    }
    None
}
```

Cargo-port supplies an `impl HitTestRegistry for Panes` that returns
`HITTABLE_Z_ORDER` (Toasts-free, see below) for `z_order()` and matches
`HittableId` variants to `&dyn Hittable` references in `pane(id)`.

### Toasts special case

`HittableId::Toasts` has no `impl Hittable` — toast hit-testing is
hardcoded in `src/tui/interaction.rs::hit_test_toasts` (lines 91–101)
because toasts are rendered by the framework, not by a cargo-port
pane struct. The plan handles this as a pre-pass: the cargo-port-side
`Panes::hit_test_at` calls `hit_test_toasts` first, then delegates to
`tui_pane::hit_test_at` for the remaining z-order:

```rust
fn hit_test_at(&self, pos: Position) -> Option<HoverTarget> {
    if let Some(t) = hit_test_toasts(self, pos) { return Some(t); }
    tui_pane::hit_test_at(self, pos)
}
```

`HITTABLE_Z_ORDER` loses its `Toasts` entry; the pre-pass owns that
layer exclusively.

### What moves

- `Hittable` trait declaration → `tui_pane/src/dispatch/mod.rs`
- `HitTestRegistry` trait + `hit_test_at` generic helper →
  `tui_pane/src/dispatch/hit_test.rs`

### What stays in `src/tui/`

- `Pane` trait (the render dispatch) — stays in `src/tui/pane/dispatch.rs`
  unchanged. Phases 4 and 5 unify pane bodies; Phase 6 moves the trait.
- `PaneRenderCtx` struct — concrete aggregate of cargo-port refs.
- `HoverTarget` enum — references cargo-port `PaneId` and `DismissTarget`.
- `HittableId` enum + `HITTABLE_Z_ORDER` array (Toasts removed from
  the const; cargo-port pane variants only).
- The `hit_test_tests` module — tests cargo-port's z-order const.
- `src/tui/interaction.rs::hit_test_toasts` — toast-specific pre-pass.

### Module re-exports

```rust
mod dispatch;
pub use dispatch::{HitTestRegistry, Hittable, hit_test_at};
```

### Sequencing

1. Add `Hittable` (associated `Ctx`, `Target`) and `HitTestRegistry`
   + `hit_test_at` to `tui_pane`. Re-export. Verify build is green —
   cargo-port code still uses its own `Hittable` trait at this point.
2. Update every `impl Hittable for X` in `src/tui/panes/pane_impls.rs`
   and `src/tui/overlays/pane_impls.rs` to use `tui_pane::Hittable`
   with `type Target = HoverTarget;`. Drop the `Pane` supertrait
   reference at each impl (the supertrait is being removed in the new
   trait). Pre-flight count: `rg "impl Hittable for" src/ | wc -l`
   should report 11 (verified: PackagePane, LangPane, CpuPane, GitPane,
   TargetsPane, ProjectListPane in `panes/pane_impls.rs`; Lint in
   `state/lint.rs`; Ci in `state/ci.rs`; KeymapPane, SettingsPane,
   FinderPane in `overlays/pane_impls.rs`).
3. Remove the `HittableId::Toasts` variant entirely from the
   `HittableId` enum in `src/tui/pane/dispatch.rs`, and remove it
   from `HITTABLE_Z_ORDER`. Toasts is fully handled by the pre-pass
   `hit_test_toasts` and never goes through the registry, so the
   variant becomes dead. The `hit_test_tests::z_order_covers_every_hittable_id`
   test passes naturally — both `HittableId::iter()` and
   `HITTABLE_Z_ORDER` shrink by one and remain equal. Also update
   `src/tui/interaction.rs` lines 64 and 71, which currently reference
   `HittableId::Toasts` in the pre-pass dispatch — those match arms
   disappear when the variant goes. Pre-flight check after:
   `rg "HittableId::Toasts" src/` should return zero matches.
4. Implement `impl HitTestRegistry for Panes` in cargo-port (likely
   in `src/tui/panes/mod.rs` or a new `dispatch.rs` next to it).
   `z_order()` returns the Toasts-free `HITTABLE_Z_ORDER`; `pane(id)`
   matches each `HittableId` variant to the corresponding pane field
   on `Panes`.
5. Rewrite cargo-port's `Panes::hit_test_at` to call
   `hit_test_toasts` first, then `tui_pane::hit_test_at(self, pos)`.
6. Delete cargo-port's `Hittable` trait declaration from
   `src/tui/pane/dispatch.rs`. Keep the `Pane` (render) trait in
   that file — Phase 6 moves it.
7. Cargo checkpoint.

### Risk note

Step 2 touches 11 impl blocks. A missed impl block fails to compile
until rewritten. Do all 11 in one pass.

The `HitTestRegistry::pane` method returns `&dyn Hittable<Target = …>`
with the associated type fully named, which works in trait-object
position. The lifetime issue that would have plagued a
`Hittable<Ctx>` generic version dissolves with associated types.

### End-state contents

After Phase 3, `src/tui/pane/` contains:
- `mod.rs` — `pub mod dismiss; pub mod dispatch;` plus re-exports of
  cargo-port-side types and pass-through re-exports of `tui_pane`
  items for call-site convenience.
- `dispatch.rs` — keeps the `Pane` (render) trait, `PaneRenderCtx`,
  `HoverTarget`, `HittableId`, and the Toasts-free `HITTABLE_Z_ORDER`.
  Phase 6 moves the render trait out.
- `dismiss.rs` — `DismissTarget` enum, unchanged.

## Phase 4 — Cargo-port-side render unification (tier-one panes)

This phase routes the easy-to-absorb tiled panes through
`Pane::render` and widens `PaneRenderCtx` once with the minimum
additional ref needed (in-flight runtime state). The harder
absorptions (`ProjectListPane` and the three overlay popups) move in
Phase 5, which widens `PaneRenderCtx` further. The render trait
rename to `Renderable` happens in Phase 6 alongside the move to
`tui_pane`.

### What this phase absorbed

| Free function | Target pane type | Outcome |
|---------------|------------------|---------|
| Targets has-data branching (inline in `render_tiled_pane`) | `TargetsPane` | Branching + both bodies absorbed into `TargetsPane::render`; `render_targets_pane_body` is the new body fn. |
| `render_lints_pane` (wrapper in `render.rs`) | `Lint` | Wrapper renamed to `dispatch_lints_render` and now calls `Pane::render(&mut app.lint, …)`. The existing `render_lints_pane_body` is unchanged. |
| `render_ci_pane` (wrapper in `render.rs`) | `Ci` | Same pattern — wrapper renamed to `dispatch_ci_render` and routes through `Pane::render`. |
| `panes::render_output_panel` (free fn in `panes/output.rs`) | `OutputPane` | Body refactored to `render_output_pane_body(frame, area, ctx)`; new `impl Pane for OutputPane` calls it. Reads come from `ctx.inflight`. |

`Panes` gains `dispatch_targets_render` and `dispatch_output_render`
helpers symmetric with the existing
`dispatch_{package,lang,cpu,git}_render` methods.
`render_tiled_pane`'s Targets / Lints / CiRuns / Output arms now
route through these dispatchers.

### What changes

- `PaneRenderCtx` gains one new field: `inflight: &Inflight`
  (used by `OutputPane::render`). `App::split_panes_for_render`,
  `split_lint_for_render`, `split_ci_for_render` widen to return
  `&Inflight` alongside the existing refs.
- `panes/output.rs::render_output_panel` is renamed to
  `render_output_pane_body` and takes `(frame, area, &PaneRenderCtx)`
  instead of `(frame, &App, area)`.
- `panes/targets.rs::render_targets_panel` /
  `render_empty_targets_panel` are dissolved into
  `render_targets_pane_body` which dispatches the has-data /
  empty branches internally.
- `render_tiled_pane` arms for Targets / Lints / CiRuns / Output now
  call `dispatch_via_trait` (or the lint/ci dispatch helpers).

### What stays put for Phase 5

- `render_left_panel(frame, app, area)` and
  `render_project_list(frame, &mut App, area)` still bypass the trait.
- The three overlay popup render functions (`render_keymap_popup`,
  `render_settings_popup`, `render_finder_popup`) still get called
  directly from cargo-port's main render function.
- `KeymapPane`, `SettingsPane`, `FinderPane`, `ProjectListPane`
  retain stub `impl Pane` bodies.

### End-state contents

After Phase 4:
- `PaneRenderCtx` has the `inflight` field.
- `TargetsPane`, `OutputPane`, `Lint`, `Ci` all render through
  `Pane::render` via their `dispatch_*_render` helpers.
- `render_tiled_pane` has bespoke calls only for `ProjectList` (now
  the single remaining bypass on the tiled path) — every other arm
  uses `dispatch_via_trait` or the lint/ci dispatchers.
- `render_targets_panel`, `render_empty_targets_panel`,
  `render_output_panel`, `render_lints_pane`, `render_ci_pane`
  are deleted (replaced by the new body fns / dispatchers).

## Phase 5 — Cargo-port-side render unification (tier-two panes)

The remaining absorptions: `ProjectListPane` (the ~96-line
`render_project_list` body) and the three overlay popups
(`KeymapPane`, `SettingsPane`, `FinderPane`). This phase widens
`PaneRenderCtx` with the additional refs these renderers need and
absorbs each body into its `Pane::render` impl.

### Why this needs its own phase

`render_project_list` reads from many App subsystems
(`Scan`, `Ci`, `Lint`, `ProjectList`, `Config`) and uses several
App-shell methods that aren't currently on subsystem types:
`app.visible_rows()`, `app.dismiss_target_for_row(row)`,
`app.lint_cell(status)`, `app.discovery_name_segments_for_path(...)`.
The overlay popups have the same problem — each reads framework /
overlay / app state that `PaneRenderCtx` doesn't carry today, and
each mutates self (the framework-owned or overlays-owned pane) during
render.

Folding all that into Phase 4 made the diff too large for a single
green commit. Phase 5 widens the context once and absorbs all four
renderers together.

### What changes

- `PaneRenderCtx` gains fields for the additional subsystem refs:
  `scan: &Scan`, `ci: Option<&Ci>`, `lint: Option<&Lint>`,
  `inline_error: Option<&str>` (from overlays).
  `ci`/`lint` are `Option` because their self-render aliases:
  when `Ci::render(&mut self, ..)` is called, the disjoint `&Ci`
  can't be supplied in the same context.
- `App::split_panes_for_render` widens to return the additional
  refs. Two new split-borrow accessors land for the overlay panes:
  one returning `&mut KeymapPane` + ctx refs, one for
  `SettingsPane`, one for `FinderPane`. Each disjoint-borrow split
  destructures `App` so the pane and its supporting refs don't
  alias.
- App methods used by `render_project_list` are refactored into
  free functions taking explicit refs from `PaneRenderCtx`:
  - `lint_cell(status, &Config, animation_elapsed) -> LintCell`
  - `discovery_name_segments_for_path(&Scan, &Config, ...)`
  - `dismiss_target_for_row(&ProjectList, row)` — already a thin
    delegator; call the inner method directly.
  - `visible_rows()` — already on `ProjectList`; use ctx ref.
- `render_project_list` becomes `render_project_list_pane_body`
  taking `(frame, area, &mut ProjectListPane, &PaneRenderCtx)`.
  `ProjectListPane::render` calls it.
- `render_left_panel` is deleted.
- The three overlay popup render functions become
  `render_keymap_pane_body`, `render_settings_pane_body`,
  `render_finder_pane_body`, taking `(frame, area, &mut Self, &PaneRenderCtx)`.
  Each overlay's `impl Pane::render` calls its body fn and
  short-circuits when the overlay isn't currently active.
- Cargo-port's main render function dispatches the overlays
  through `Pane::render` via dedicated dispatch helpers (or
  collapses them into the tiled-render loop entry).

### Sequencing

1. Widen `PaneRenderCtx` with the additional ref fields and update
   every existing destructure / construction site. Verify the build
   is green with no functional change yet.
2. Refactor the App methods used by `render_project_list` into free
   functions on the relevant subsystem (or free fns taking explicit
   ref args).
3. Absorb `render_project_list` into `ProjectListPane::render`.
   Delete `render_left_panel`. Update `render_tiled_pane`'s
   `PaneId::ProjectList` arm to `dispatch_via_trait`.
4. Absorb each overlay popup body into its `Pane::render` impl.
   Add the visibility short-circuit at the top of each body.
   Update cargo-port's main render function to route the overlays
   through their trait impls.
5. Cargo checkpoint.

### Risk notes

- **Behavioral risk is real.** Each absorption moves code that runs
  every frame. Run the app in dev between absorptions and compare
  visually — these renderers handle empty states, shimmer overlays,
  and conditional layouts that can break in subtle ways. The cargo
  checkpoint catches type errors but not visual regressions.
- **`render_project_list` mutates `app.project_list.set_cursor(...)`
  during render** — the line near the end that syncs cursor with
  ratatui's `ListState::selected()`. Phase 5 should either drop
  this (it appears to be a no-op since `selected` is what we set
  it to) or expose a narrow setter through ctx. Verify behavior is
  unchanged after the drop.
- **Overlay render functions mutate viewport state during render.**
  `render_keymap_popup` writes to `app.framework.keymap_pane.viewport_mut()`;
  `render_settings_popup` writes to `app.framework.settings_pane.viewport_mut()`;
  `render_finder_popup` writes to `app.overlays.finder_pane.viewport_mut()`.
  When absorbed into `impl Pane::render(&mut self, frame, area, ctx)`,
  the mutations must move to `self` (the pane struct owns the viewport).
  Before each overlay absorption, grep for `app\.framework\.<pane>` and
  `app\.overlays\.<pane>` in the render function to enumerate every
  mutation path that needs re-routing.

### End-state contents (as shipped)

After Phase 5:
- `src/tui/render.rs::render_tiled_pane` — every arm calls
  `dispatch_via_trait` (`ProjectList` included; previous bespoke
  `render_left_panel` deleted).
- `src/tui/panes/pane_impls.rs` — `ProjectListPane` has a real
  `impl Pane` body that calls
  `project_list::render_project_list_pane_body`. `state/lint.rs`
  and `state/ci.rs` already had real bodies from Phase 4.
- `src/tui/overlays/pane_impls.rs` — `FinderPane` has a real
  `impl Pane` body that calls `finder::render_finder_pane_body`.
  `KeymapPane` and `SettingsPane` still have no-op `impl Pane`
  bodies; their popup rendering remains in
  `keymap_ui::render_keymap_popup` / `settings::render_settings_popup`,
  invoked through `dispatch_keymap_overlay` /
  `dispatch_settings_overlay` helpers in `render.rs`. Absorbing
  those two into the trait would require either widening
  `PaneRenderCtx` with `&FrameworkKeymap<App>` (which carries the
  `App` generic — entangling the trait with cargo-port's own
  type) or refactoring the keymap-row builders in `keymap_ui` to
  drop their `&App` dependency. Deferred to a follow-up phase.
- `PaneRenderCtx` gained `scan: &Scan`, `ci: Option<&Ci>`,
  `lint: Option<&Lint>`, `inline_error: Option<&str>`. Tile-pane
  dispatchers populate every field; the lint and inline_error
  fields are currently consumed only by tests and are reserved
  for the deferred Keymap / Settings absorption (see `#[allow]`
  on the field docs in `src/tui/pane/dispatch.rs`).
- App methods that the legacy ProjectList renderer used —
  `App::lint_cell`, `App::discovery_name_segments_for_path`,
  `App::dismiss_target_for_row` — are now either `#[cfg(test)]`
  thin delegators (the first two) or removed entirely (the last,
  reachable via `ProjectList::dismiss_target_for_row_inner`).
  The production renderer calls free fns:
  `tui::state::lint_cell_for(status, &Config, animation_elapsed)`
  and
  `tui::app::discovery_name_segments_for_path_with_refs(&Scan, &Config, &ProjectList, ...)`.
- Cargo-port's main render function — every tile pane and the
  Finder overlay render through `Pane::render`. The Keymap and
  Settings overlays render through named `dispatch_*_overlay`
  helpers in `render.rs` rather than trait dispatch; the
  helpers exist so `ui()` is uniform across overlay entry
  points even though the bodies haven't been absorbed yet.
  (Updated: Phase 6.11 / 6.12 absorbed those overlay bodies
  into `Renderable::render`. See Phase 6 end-state.)

## Phase 6 — `Renderable` trait and render dispatch loop

With cargo-port unified after Phases 4 and 5, the render trait can
move to `tui_pane` and the dispatch loop becomes generic. Symmetric
with the `Hittable` + `HitTestRegistry` + `hit_test_at` stack from
Phase 3.

### Trait design

In `tui_pane/src/dispatch/mod.rs`:

```rust
pub trait Renderable<Ctx> {
    fn render(&mut self, frame: &mut Frame<'_>, area: Rect, ctx: &Ctx);
}
```

`Ctx` is a generic parameter, not a generic associated type — GATs
make traits non-object-safe (Rust's rule), and the dispatch loop
needs `&mut dyn Renderable<…>` to work. Cargo-port impls fix it:
`impl Renderable<PaneRenderCtx<'_>> for X`.

### Dispatch loop

In `tui_pane/src/dispatch/render.rs`:

```rust
pub trait PaneRegistry {
    type PaneId: Copy;
    type Ctx<'a>;
    fn pane_mut(&mut self, id: Self::PaneId)
        -> Option<&mut dyn for<'a> Renderable<Self::Ctx<'a>>>;
}

pub fn render_panes<R: PaneRegistry>(
    frame: &mut Frame<'_>,
    registry: &mut R,
    layout: &ResolvedPaneLayout<R::PaneId>,
    ctx: &R::Ctx<'_>,
) {
    for resolved in &layout.panes {
        if let Some(pane) = registry.pane_mut(resolved.pane) {
            pane.render(frame, resolved.area, ctx);
        }
    }
}
```

(Shipped version. The original draft took `layout: &[(R::PaneId,
Rect)]` — switched to `ResolvedPaneLayout` so the embedding hands
in the same layout type it already builds for input dispatch.)

The `for<'a>` in `pane_mut`'s return type is a higher-ranked trait
bound (HRTB). It says "this trait object works for any lifetime of
`Ctx`, not a specific one." Required because `Ctx<'a>` is a GAT on
the registry — elided lifetimes don't work in trait-object position.
The HRTB is the workaround. One unusual type in one method signature;
every other site (the impls, the dispatch call) reads as normal Rust.

The trait-object coercion has been verified compile-OK: writing
`impl Renderable<PaneRenderCtx<'_>> for PackagePane` produces a
universally-quantified impl that satisfies the HRTB bound, and the
match arms in `pane_mut` (`Some(&mut self.package)`) coerce cleanly.

Cargo-port supplies `impl PaneRegistry for Panes`. Same registry
struct as `HitTestRegistry` from Phase 3.

### What moves

- `Pane` trait declaration from `src/tui/pane/dispatch.rs` →
  `tui_pane/src/dispatch/mod.rs` as `Renderable` (rename happens
  during the move).
- `dispatch_via_trait` (currently in `src/tui/render.rs`) — the
  closure-based dispatch helper folds into the generic `render_panes`.
- `render_tiled_pane`'s match dissolves entirely; cargo-port's main
  render orchestration becomes one call to `tui_pane::render_panes`
  followed by `tui_pane::render_toasts`.

### What stays in `src/tui/`

- `PaneRenderCtx` struct — still cargo-port domain.
- `Panes::render_tiled_panes` thin wrapper around the generic loop
  (cargo-port side), if useful for ergonomics.

### Module re-exports

```rust
pub use dispatch::{PaneRegistry, Renderable, render_panes};
```

### Sequencing

1. Add `Renderable` and `PaneRegistry` to `tui_pane`. Add
   `render_panes`. Re-export. Verify build is green — cargo-port
   still uses its own `Pane` trait at this point.
2. Rename cargo-port impls. Change every `impl Pane for X` to
   `impl tui_pane::Renderable<PaneRenderCtx<'_>> for X`. After
   Phase 5 there should be 12 impl sites: PackagePane, LangPane,
   CpuPane, GitPane, TargetsPane, ProjectListPane, OutputPane in
   `panes/pane_impls.rs`; Lint in `state/lint.rs`; Ci in
   `state/ci.rs`; KeymapPane, SettingsPane, FinderPane in
   `overlays/pane_impls.rs`. Verify with `rg "impl Pane for" src/`
   beforehand.
3. Add `impl PaneRegistry for Panes` (likely on the same registry
   struct used by `HitTestRegistry` from Phase 3). `pane_mut(id)`
   matches each `PaneId` variant to a `&mut dyn Renderable<…>`
   reference.
4. Replace `render_tiled_pane`'s match with one call to
   `tui_pane::render_panes(frame, &mut registry, &layout, &ctx)`.
5. Delete `src/tui/pane/dispatch.rs::Pane` (now moved). Decide
   whether the file is empty enough to delete entirely (everything
   else moved in Phase 3 already).
6. Cargo checkpoint.

### Risk notes

- **Sequencing with Phase 3's `HitTestRegistry`.** Both impls live
  on the same `Panes` struct. Plan to declare both traits on the
  same registry impl. The registry type itself may need fields
  added during Phase 6 — that's fine, as long as Phase 3's
  checkpoint was green at the time.

### End-state contents (as shipped)

After Phase 6:

- **`tui_pane/src/dispatch/`** contains:
  - `mod.rs` declares `Hittable`, `Renderable`, `HitTestRegistry`,
    `PaneRegistry`.
  - `hit_test.rs` — generic hit-test dispatch loop (from Phase 3).
  - `render.rs` — `Renderable<Ctx>` trait, `PaneRegistry` trait with
    GAT `Ctx<'a>` and HRTB `for<'a>` on `pane_mut`, `render_panes`
    generic function that walks a `ResolvedPaneLayout<R::PaneId>`.

- **`PaneRenderCtx` (cargo-port)** is uniform across all panes in
  one frame's render pass. Per-pane state moved out of ctx onto
  the panes themselves:
  - `focus_state` / `is_focused` → each pane carries a
    `pub focus: tui_pane::RenderFocus` field, stamped by
    `sync_pane_focus` before the tile loop and by the overlay
    dispatchers before their dispatch.
  - `ci: Option<&Ci>` → replaced by `ci_status_lookup: &CiStatusLookup`,
    an owned snapshot built via `Ci::status_lookup()` before the
    split-borrow runs. `ProjectListPane` reads CI status per row
    through this lookup instead of `&Ci`, which frees the CI
    pane's own dispatcher to consume `&mut self.ci`.
  - `lint: Option<&Lint>` → removed entirely. The Lints pane reads
    its own data via `&mut self`; no other pane consumed it.

- **`PaneRenderCtx`** carries: `animation_elapsed`, `config`,
  `project_list`, `selected_project_path`, `inflight`, `scan`,
  `ci_status_lookup`, plus `keymap_render_inputs` and
  `settings_render_inputs` (both `Option`, populated only by their
  own overlay dispatcher).

- **`KeymapRenderInputs`** (in `tui/keymap_ui`) and
  **`SettingsRenderInputs`** (in `tui/settings`) are owned
  precomputed snapshots built before `App::split_for_render`.
  Their constructors (`prepare_keymap_render_inputs` /
  `prepare_settings_render_inputs`) walk the still-current `&App`
  to build the row lines, popup-width hints, and selectable
  counts. Each is plumbed into ctx as `Option<&'a …>` and read
  inside the matching pane's `Renderable::render` body.

- **`App::split_for_render`** is the single render-time
  split-borrow entry point. It destructures `App`'s pane-owning
  fields and returns `RenderBorrows { registry: RenderRegistry,
  ctx: PaneRenderCtx }`. The four pre-Phase-6 split helpers
  (`split_panes_for_render`, `split_ci_for_render`,
  `split_lint_for_render`, plus `DispatchArgs`/`build_ctx` in
  `panes/system.rs`) are deleted.

- **`RenderRegistry<'a>`** holds disjoint `&mut` refs to every
  renderable cargo-port pane: `PackagePane`, `LangPane`,
  `CpuPane`, `GitPane`, `TargetsPane`, `ProjectListPane`,
  `OutputPane`, `Lint`, `Ci`, plus framework-owned `KeymapPane`
  and `SettingsPane` (reached via `app.framework.keymap_pane` /
  `app.framework.settings_pane`). The `Finder` overlay stays
  out — sized off the whole frame, not part of the tile layout.

- **`impl tui_pane::PaneRegistry for RenderRegistry`** maps each
  `PaneId` to its `&mut dyn for<'ctx> Renderable<PaneRenderCtx<'ctx>>`.
  `Toasts` and `Finder` return `None` from `pane_mut`.

- **All twelve `Renderable<PaneRenderCtx<'_>>` impls** live in
  cargo-port: tile panes in `panes/pane_impls.rs`, `Lint` in
  `state/lint.rs`, `Ci` in `state/ci.rs`, framework-owned
  overlays in `overlays/pane_impls.rs` (which delegate to body
  fns in `keymap_ui` and `settings`).

- **`src/tui/render.rs::ui`** now reads, in order:
  1. `sync_pane_focus(app)` — stamps each pane's `focus` field.
  2. `app.ci.status_lookup()` — builds the owned CI snapshot.
  3. `app.split_for_render(...)` — produces `RenderBorrows`.
  4. `tui_pane::render_panes(frame, &mut split.registry, &tiled,
     &split.ctx)` — dispatches every tile pane through the trait.
  5. `tui_pane::render_toasts(...)` — post-pass (still outside
     `Renderable`; see "Toasts asymmetry" below).
  6. `dispatch_keymap_overlay` / `dispatch_settings_overlay` /
     `dispatch_finder_render` — overlay dispatchers that
     precompute their own inputs, split, and call
     `Renderable::render` directly on the relevant registry
     entry.

- **`src/tui/pane/dispatch.rs`** — kept (not deleted). Holds
  `PaneRenderCtx`, `HoverTarget`, `HittableId`, and
  `HITTABLE_Z_ORDER`. The `Pane` trait is gone; cargo-port-side
  refs use `tui_pane::Renderable` directly.

- **`render_tiled_pane`, `dispatch_via_trait`,
  `dispatch_lints_render`, `dispatch_ci_render`, `DispatchArgs`,
  `build_ctx`, and the seven `Panes::dispatch_*_render`
  methods** are deleted.

## Post-Phase-6 — `Renderable` design review (resolved)

The trait + registry shipped through all of cargo-port's panes
and both framework-owned overlay popups. Resolution of the
review questions:

- **Does `type Ctx<'a>` justify its cost?** Yes. The GAT is what
  lets the registry's `&mut Pane` borrows and the ctx's `&` borrows
  carry independent lifetimes. Cargo-port instantiates with
  `type Ctx<'ctx> = PaneRenderCtx<'ctx>`, and the HRTB on
  `pane_mut`'s return type lets a single `&mut dyn Renderable`
  trait object work across any ctx lifetime. `render_panes`
  builds its own `ctx: &Ctx<'_>` per call, never tied to the
  registry's borrow — which is exactly the property the
  split-borrow design needs.

- **Is `PaneRegistry::pane_mut` ergonomic in practice?**
  Acceptable. The `impl PaneRegistry for RenderRegistry` match is
  one arm per `PaneId` variant returning a `&mut dyn Renderable`
  trait object. It's mechanical but readable; no surprising
  trait-object coercion errors after the initial prototype
  confirmed the HRTB design.

- **Is it acceptable that framework-owned overlay panes
  (`KeymapPane`, `SettingsPane`, `FinderPane`) impl `Renderable`
  via cargo-port-side `impl` blocks?** Yes — the cross-crate
  `impl Renderable<PaneRenderCtx<'_>> for KeymapPane` satisfies
  the orphan rule because `PaneRenderCtx` is a local generic
  parameter. No need to move trait bodies into `tui_pane`.

- **Should the framework's `Toasts` render go through
  `Renderable` too?** Identified as the remaining asymmetry —
  `Toasts` is rendered via `tui_pane::render_toasts` as a
  post-pass in `render::ui`, not through the trait. Folding it
  in is tracked as Phase 6.14 below.

- **Did `HitTestRegistry` and `PaneRegistry` collapse into one
  trait?** No. They live on different types (`HitTestRegistry`
  on `App`, `PaneRegistry` on `RenderRegistry<'_>` because the
  render path needs disjoint `&mut` borrows that `App`-as-a-whole
  can't supply). Collapsing them would require both traits to
  operate on the same borrow tier, which the split-borrow design
  prevents. Leaving them split.

## Phase 6.7 – 6.12 — Sub-phases that shipped during Phase 6

The Phase 6 work expanded beyond the original three-step
sequence as cargo-port's split-borrow patterns made themselves
felt. The actual sub-phases that landed:

- **6.7 — Add `PaneRegistry` + `render_panes` to `tui_pane`.**
  Trait with GAT `Ctx<'a>` and HRTB `for<'a>` on `pane_mut`'s
  return; `render_panes` walks `ResolvedPaneLayout<R::PaneId>`.

- **6.8 — Restructure `PaneRenderCtx` for uniform-loop dispatch.**
  Moved `focus_state` / `is_focused` onto each pane as a
  `RenderFocus` field. Dropped `ci`/`lint` from ctx; introduced
  `CiStatusLookup` owned snapshot built via `Ci::status_lookup()`.
  Renamed `ProjectList::ci_status_for` →
  `ci_status_using_lookup` (and the `_root_item` variant).

- **6.9 — Build `RenderRegistry` + `impl PaneRegistry` on
  cargo-port side.** Single split-borrow accessor
  `App::split_for_render` returning
  `RenderBorrows { registry, ctx }`.

- **6.10 — Replace `render_tiled_pane` with `render_panes`.**
  Deleted the per-pane match, all `Panes::dispatch_*_render`
  methods, `DispatchArgs`, `build_ctx`, `dispatch_via_trait`,
  `dispatch_lints_render`, `dispatch_ci_render`. Tile loop is
  now one `tui_pane::render_panes` call.

- **6.11 — Absorb the Keymap overlay into `Renderable`.** Added
  `KeymapRenderInputs` owned snapshot, `prepare_keymap_render_inputs`
  builder. `render_keymap_popup` → `render_keymap_pane_body`
  signature `(frame, area, &mut KeymapPane, &PaneRenderCtx)`.
  `KeymapPane::render` impl no longer no-op. `RenderRegistry`
  gained `keymap_pane: &'a mut KeymapPane`. `KeymapPane` (in
  `tui_pane`) gained a `pub focus: RenderFocus` field.

- **6.12 — Absorb the Settings overlay into `Renderable`.**
  Same pattern as Keymap: `SettingsRenderInputs`,
  `prepare_settings_render_inputs`, body fn,
  `SettingsPane::render` impl wired up, registry field added,
  `SettingsPane.focus` field added in `tui_pane`.

## Phase 6.13 — Post-commit cleanups (planned)

Small follow-ups identified after Phase 6.12 shipped:

- Remove the stale `#[allow(dead_code)]` on
  `PaneRenderCtx::inline_error` — the absorption shipped, the
  reason no longer applies. Delete the field if it's still
  unread, or wire its consumer if a reader exists.
- Drop the unused `area` parameter from
  `render_settings_pane_body` — the popup centers itself off
  `frame.area()` via `PopupFrame::render_with_areas`.
- Sync this document's Phase 6 end-state with what actually
  shipped (i.e. the rewrite this section sits in).

## Phase 6.14 — Fold Toasts into `Renderable` (planned)

The only remaining pane-like thing not going through the trait.
`Toasts` is framework-owned and currently rendered via
`tui_pane::render_toasts(...)` as a post-pass in `render::ui`.

Folding `Toasts` into `Renderable` would add an
`impl Renderable<Ctx> for Toasts` inside `tui_pane` itself —
closing the "no internal consumers of `Renderable`" gap. The
context type needs design: `Toasts` reads toast settings, the
active-toast list, the focused-toast id, and the focused-pane
flag — none of which fit cargo-port's `PaneRenderCtx`. Two
options on the table:

1. **Framework-local ctx type.** Define
   `tui_pane::ToastsRenderCtx<'a>` carrying just what `Toasts`
   reads. `Toasts: Renderable<ToastsRenderCtx<'_>>`. Embedding
   builds the ctx and calls `Renderable::render` directly (no
   registry — toasts aren't keyed by `PaneId`).
2. **Generic Ctx with framework-side impl.** Keep ctx
   embedding-defined; require embeddings to implement a small
   `ToastsContext` trait that `Toasts::render` reads through.
   More flexible, more boilerplate.

Option 1 is simpler and matches cargo-port's usage pattern.

## Deferred (not in this plan)

These are reusable in principle but require larger generalization that
makes them their own project:

- `src/tui/finder/` — wired to project search via `FinderItem`,
  `FinderKind`, `build_finder_index`. A generic finder needs an abstract
  `Searchable` trait and a way to express per-item kinds. Worth a
  dedicated plan after the four phases above.
- `src/tui/keymap_ui/` — popup UI is generic in concept but currently
  references cargo-port action enums and the cargo-port `Settings`
  struct. Generalization is possible but would touch ~10 trait bounds.
- `src/tui/render.rs`, `src/tui/terminal.rs`, `src/tui/background.rs` —
  app glue.
- The big domain panes (`src/tui/panes/lints.rs`, `package.rs`, `ci.rs`,
  `git.rs`, etc.) — domain-specific data renderers. Stay.
- System metric samplers (CPU, disk, memory, network) — app-domain
  code, not framework. `src/tui/cpu.rs` stays in cargo-port for the
  same reason GitHub and crates.io integrations stay: `tui_pane` is
  the ratatui pane framework, not a kitchen-sink of decided
  integrations.
