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
| 4 | Cargo-port-side render unification (no tui_pane changes) | None | Medium | 5 tiled + 3 overlay renderers absorbed into `impl Pane` blocks, one commit |
| 5 | `Renderable` trait + render dispatch loop | Generic `Ctx` param + HRTB on registry, new `PaneRegistry` trait | Medium | 1 trait moved, 11 impl sites renamed, render orchestration switches to generic loop, one commit |

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
│   ├── mod.rs                ← Hittable trait (Phase 3) + Renderable trait + PaneRegistry (Phase 5)
│   ├── hit_test.rs           ← generic hit-test dispatch loop (Phase 3)
│   └── render.rs             ← generic render dispatch loop (Phase 5)
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
trait — that's Phase 4's job. The trait move itself happens in Phase 5.

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
  unchanged. Phase 4 unifies pane bodies; Phase 5 moves the trait.
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
   that file — Phase 5 moves it.
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
  Phase 5 moves the render trait out.
- `dismiss.rs` — `DismissTarget` enum, unchanged.

## Phase 4 — Cargo-port-side render unification

Before the render trait can move to `tui_pane`, every cargo-port pane
needs to funnel through a single render trait with a real body — no
free-function bypasses, no stub impls that defer to popup-render code
paths. This phase does that unification entirely inside cargo-port,
with zero `tui_pane` involvement.

The render trait stays named `Pane` in cargo-port for this phase; the
rename to `Renderable` happens in Phase 5 alongside the move.

### Why this needs its own phase

Cargo-port currently splits render dispatch across two paths:

- **Tiled-pane dispatch** in `src/tui/render.rs::render_tiled_pane`
  (lines 416–461). Of nine tiled-pane arms: 4 call `dispatch_via_trait`
  (Package, Git, Lang, Cpu) — these use the `Pane` trait. 5 are
  bespoke free functions (`render_left_panel`, Targets branching,
  `render_lints_pane`, `render_ci_pane`, `render_output_panel`).
- **Overlay dispatch** runs separately in cargo-port's main render
  function. Keymap, Settings, and Finder are rendered by calling
  `render_keymap_popup`, `render_settings_popup`, `render_finder_popup`
  (or equivalent) conditionally on overlay activation. Their pane
  structs have stub `impl Pane` bodies.

Phase 5's generic dispatch loop requires every pane to render through
the trait. This phase makes that true for cargo-port without touching
`tui_pane`.

Toasts stays outside `Renderable` entirely — its render is fully
framework-owned in `tui_pane::toasts` and runs as a separate pass
after the main render loop in Phase 5.

### Unification work

Eight pane types have stub `impl Pane for X` blocks (or bypass the
trait entirely via free functions). Unification fills in real bodies:

**Tiled panes (5):**

| Free function | Lines (approx) | Target pane type | Location of stub impl |
|---------------|---------------|------------------|-----------------------|
| `render_left_panel` (2-line wrapper) → `panes::render_project_list` | ~96 (in `src/tui/panes/project_list.rs:127`) | `ProjectListPane` | `pane_impls.rs:350` |
| Targets has-data branching (inline in `render_tiled_pane`) | ~10 (inline at `render.rs:447-455`) | `TargetsPane` | `pane_impls.rs:321` |
| `render_lints_pane` | ~20 (in `render.rs:378`) | `Lint` (the state type) | `state/lint.rs:214` |
| `render_ci_pane` | ~20 (in `render.rs:397`) | `Ci` (the state type) | `state/ci.rs:271` |
| `panes::render_output_panel` | ? (in `src/tui/panes/output.rs`) | `OutputPane` | `OutputPane` (`pane_impls.rs:372`) currently has **no** `impl Pane` — Phase 4 adds one, absorbing `render_output_panel`'s body. Verify before unification that no later refactor added a stub impl. |

**Overlays (3):**

| Render function | Target pane type | Location of stub impl | Absorbed logic |
|-----------------|------------------|-----------------------|----------------|
| `render_keymap_popup` (find the actual location) | `KeymapPane` | `overlays/pane_impls.rs:24` | Visibility check + centered area + popup body |
| `render_settings_popup` (find the actual location) | `SettingsPane` | `overlays/pane_impls.rs:47` | Visibility check + centered area + popup body |
| `render_finder_popup` (find the actual location) | `FinderPane` | `overlays/pane_impls.rs:70` | Visibility check + centered area + popup body |

Each overlay's render method short-circuits when the overlay isn't
active, computes its own centered area from `frame.area()`, and
draws.

Note: `Lint` and `Ci` are state types (not `LintsPane`/`CiRunsPane`)
that double as the pane types for those tiled cells. They already
have `impl Pane` blocks; the rename to a dedicated pane struct, if
any, is out of scope.

### What changes

- Free render functions absorbed into pane `impl Pane` bodies (5 tiled
  + 3 overlay = 8 functions absorbed and deleted).
- `render_tiled_pane`'s match collapses: every arm now calls
  `dispatch_via_trait` (no bespoke calls).
- Cargo-port's main render function stops calling
  `render_keymap_popup`, `render_settings_popup`, `render_finder_popup`
  directly — those overlays now render through the trait dispatch
  alongside tiled panes (visibility short-circuit inside each impl).

### What stays put

- `Pane` trait in `src/tui/pane/dispatch.rs` (renamed in Phase 5).
- `PaneRenderCtx` struct — unchanged.
- `tui_pane` — no changes this phase.

### Sequencing

1. For each of the 8 panes in the unification tables, absorb the
   render logic from the free function (or popup function) into the
   pane's `impl Pane`. Five iterations for tiled panes, three for
   overlays. Order suggested: do the simplest ones first (Targets
   branching, the overlay visibility checks) to gain confidence,
   then tackle `render_project_list` (the largest absorption).
2. Delete each absorbed free function.
3. Update `render_tiled_pane` so every arm uses `dispatch_via_trait`.
4. Update cargo-port's main render function to dispatch overlays
   through the same trait path as tiled panes.
5. Cargo checkpoint.

### Risk notes

- **Behavioral risk is real.** Each absorption moves code that runs
  every frame. Run the app in dev between absorptions and compare
  visually — these renderers handle empty states, shimmer overlays,
  and conditional layouts that can break in subtle ways. The cargo
  checkpoint catches type errors but not visual regressions.
- **`dispatch_via_trait` signature compatibility.** Confirm before
  step 1 that the current `dispatch_via_trait` threads `PaneRenderCtx`
  through correctly. Note: Phase 2 already moves `layout_cache` onto
  `Framework` and `project_list_body` onto `ProjectListPane`, so the
  absorbed `render_project_list` no longer needs `app.layout_cache` —
  it writes `self.body_rect` and `app.framework.set_tiled_layout(...)`
  instead. Verify the remaining App-state accesses (cursor mutation,
  cached widths) can flow through `PaneRenderCtx` as-is or need a
  small widening.
- **`render_project_list` is the largest absorption (~96 lines).**
  Schedule it as its own session if needed; it accesses `&mut App`
  state that `PaneRenderCtx` doesn't currently carry.
- **Overlay render functions mutate viewport state during render.**
  `render_keymap_popup` writes to `app.framework.keymap_pane.viewport_mut()`;
  `render_settings_popup` writes to `app.framework.settings_pane.viewport_mut()`;
  `render_finder_popup` writes to `app.overlays.finder_pane.viewport_mut()`.
  When absorbed into `impl Pane::render(&mut self, frame, area, ctx)`,
  the mutations must move to `self` (the pane struct owns the viewport).
  Before each overlay absorption, grep for `app\.framework\.<pane>` and
  `app\.overlays\.<pane>` in the render function to enumerate every
  mutation path that needs re-routing.

### End-state contents

After Phase 4:
- `src/tui/render.rs::render_tiled_pane` — every arm calls
  `dispatch_via_trait`. No bespoke render functions remain in the file.
- `src/tui/panes/pane_impls.rs`, `state/lint.rs`, `state/ci.rs`,
  `overlays/pane_impls.rs` — every pane has a real `impl Pane` body.
  No stubs.
- `src/tui/pane/dispatch.rs` — unchanged; still holds the `Pane` trait.
- Cargo-port's main render function — overlays render through the
  same trait dispatch as tiled panes.

## Phase 5 — `Renderable` trait and render dispatch loop

With cargo-port unified in Phase 4, the render trait can move to
`tui_pane` and the dispatch loop becomes generic. Symmetric with the
`Hittable` + `HitTestRegistry` + `hit_test_at` stack from Phase 3.

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
    layout: &[(R::PaneId, Rect)],
    ctx: &R::Ctx<'_>,
) {
    for (id, area) in layout {
        if let Some(pane) = registry.pane_mut(*id) {
            pane.render(frame, *area, ctx);
        }
    }
}
```

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
   Phase 4 there should be 11 impl sites: PackagePane, LangPane,
   CpuPane, GitPane, TargetsPane, ProjectListPane, OutputPane in
   `panes/pane_impls.rs`; Lint in `state/lint.rs`; Ci in
   `state/ci.rs`; KeymapPane, SettingsPane, FinderPane in
   `overlays/pane_impls.rs`. Verify with `rg "impl Pane for" src/`
   beforehand; expect total of 11.
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
  added during Phase 5 — that's fine, as long as Phase 3's
  checkpoint was green at the time.

### End-state contents

After Phase 5:
- `src/tui/pane/dispatch.rs` — deleted (or trimmed to nothing).
- `src/tui/render.rs::render_tiled_pane` — replaced by a call to
  `tui_pane::render_panes`. Cargo-port's main render orchestration
  reduces to: build registry, call `render_panes`, call
  `render_toasts`.
- All `impl Pane` in cargo-port — now `impl Renderable<PaneRenderCtx<'_>>`.
- `tui_pane/src/dispatch/` — `mod.rs` declares `Renderable`,
  `Hittable`, `PaneRegistry`, `HitTestRegistry`. `hit_test.rs` and
  `render.rs` carry the two generic loops.

## Post-Phase-5 — `Renderable` design review

After Phase 5 lands, hold an explicit review pass on the `Renderable`
trait and the `PaneRegistry` abstraction before declaring this plan
complete. The trait design is the most novel piece of this extraction
and may want refinement once it has been exercised by all 11 panes
and the generic loop.

Questions to answer in the review:

- **Does `type Ctx<'a>` justify its cost?** Every pane in cargo-port
  sets it to `PaneRenderCtx<'a>`. If there's no realistic second
  context type, an associated type adds complexity without payoff —
  the alternative is a generic `Renderable<Ctx>` with a fixed
  lifetime newtype, or even a non-generic trait that takes
  `&PaneRenderCtx<'_>` directly (which then forces the trait to live
  in cargo-port, undoing Phase 5).
- **Is `PaneRegistry::pane_mut` ergonomic in practice?** The
  trait-object signature is unusual. If impl sites end up writing
  large match statements that handle every `PaneId` variant
  by-hand, the generic-loop savings may not justify the abstraction
  cost.
- **Is it acceptable that framework-owned overlay panes
  (`KeymapPane`, `SettingsPane`, `FinderPane`) impl `Renderable`
  via cargo-port-side `impl` blocks?** The trait sits in `tui_pane`
  but the impls live in cargo-port because they reference
  cargo-port's `PaneRenderCtx`. Note whether a future framework
  refactor should move the trait body / context type into `tui_pane`
  so the impls can live in the same crate as the structs.
- **Should the framework's `Toasts` render go through `Renderable`
  too?** Currently `Toasts` stays separate (rendered via
  `tui_pane::render_toasts` as a post-pass). If `Toasts` fit the
  `Renderable` contract, the framework would gain an internal
  consumer of its own trait — closing the "no internal consumers"
  asymmetry.
- **Did `HitTestRegistry` and `PaneRegistry` collapse into one
  trait?** If both are implemented on the same struct (`Panes`)
  with the same `PaneId` type, a single `Registry` trait with both
  `pane` and `pane_mut` methods plus z-order may be the right
  unification.

The review produces either "design holds, plan complete" or a list
of follow-up changes scoped as a separate post-extraction refinement.
Record the outcome in this doc and link the refinement plan if one
is needed.

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
