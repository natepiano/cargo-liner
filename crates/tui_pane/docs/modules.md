# tui_pane restructure plan

`tui_pane/src` carries seventeen top-level `.rs` files, most of which exist because the snake_case prefix is doing the work a directory should do: five `pane_*` files are an unannounced `pane/` submodule, and three layout primitives form an unannounced `layout/` submodule. The plan introduces those two directories, extracts test bulk from `keymap/builder/mod.rs`, and splits the two production-heavy files in `toasts/`. Two cross-module coupling concerns and one borderline test-extraction are noted as deferred items.

## Phase overview

| Phase | What | Risk | Rough size |
|-------|------|------|------------|
| 1 | Create `pane/` submodule (7 files in via `pane/mod.rs`) + rename `panes/` → `overlays/` | Low | 8 file/dir relocations, ~18 lib.rs paths updated, one commit |
| 2 | Create `layout/` submodule; move `viewport.rs` + `column_widths.rs` into it | Low | 2 files relocated, ~8 lib.rs re-export paths updated, one commit |
| 3 | Extract tests from `keymap/builder/mod.rs` into `keymap/builder/tests.rs` | Low | ~1198 test lines relocated, one commit |
| 4 | Lift `Toasts<Ctx>` into `toasts/mod.rs`; split `toasts/stack.rs` (616 prod) across 4 new toasts-level siblings | Medium | Internal-only callers, one commit |
| 5 | Split `toasts/render.rs` (587 prod) into a `render/` submodule | Medium | Internal-only callers, one commit |

> Line ranges named inside Phases 4 and 5 reference the file snapshot taken when this plan was written. The file has continued to grow; when executing, locate each item by **function/type name** rather than line number. The submodule assignments and dependency order remain correct.

## Phase 1 — Create `pane/` submodule (+ rename `panes/` → `overlays/`)

Seven files at the root carry the `pane_` prefix or are concrete pane chrome. They become a `pane/` directory with `pane/mod.rs` as the parent (crate convention is `mod.rs`, never the Rust 2018 `<dir>.rs` + `<dir>/` layout). In the same phase, the sibling `panes/` directory renames to `overlays/` so the singular/plural ambiguity does not arise. The structs inside keep their `*Pane` names (`KeymapPane`, `SettingsPane`) — they participate in the `Pane` trait and that name belongs on them.

### Target layout

```
tui_pane/src/
├── pane/
│   ├── mod.rs           # Pane<Ctx> trait + Mode<Ctx> enum + ModeQuery (current pane.rs contents)
│   ├── chrome.rs        # was pane_chrome.rs — PaneChrome, default_pane_chrome, empty_pane_block
│   ├── id.rs            # was pane_id.rs — FocusedPane, FrameworkFocusId, FrameworkOverlayId
│   ├── popup.rs         # was popup.rs — PopupFrame, PopupAreas, centered_rect
│   ├── rules.rs         # was rules.rs — PaneRule, RuleTitle, render_horizontal_rule, render_rules
│   ├── state.rs         # was pane_state.rs — PaneFocusState, PaneSelectionState, RenderFocus, helpers
│   └── title.rs         # was pane_title.rs — PaneTitleCount, PaneTitleGroup, pane_title, prefixed_pane_title
└── overlays/            # was panes/ — directory rename only; KeymapPane and SettingsPane keep their names
    ├── mod.rs
    ├── keymap.rs
    └── settings.rs
```

### Rationale

- `pane_chrome`, `pane_id`, `pane_state`, `pane_title` are an unannounced submodule. Each name reads `pane::<noun>` once the directory exists.
- `popup` and `rules` are pane chrome too: `PopupFrame` is an overlay frame with a `PaneChrome`-style API, and `PaneRule` draws dividers around panes. Both join the `pane/` group.
- The existing `pane.rs` becomes `pane/mod.rs` with `mod chrome; mod id; mod popup; mod rules; mod state; mod title;` at the top and the rest of its contents unchanged.
- Renaming the sibling `panes/` directory to `overlays/` removes the singular/plural collision before it appears. The binary already uses `src/tui/overlays/` for the same concept, so this aligns vocabulary. The structs `KeymapPane` and `SettingsPane` stay — they implement the `Pane` trait, and the name belongs on them.

### lib.rs re-export paths

The crate-root `pub use` paths shift from `<flat>::X` to `pane::<sub>::X`. External consumers do not see the move (they import `tui_pane::PaneChrome` etc. through the crate-root re-export, which the plan keeps intact).

```rust
// Before
pub use pane::Mode;
pub use pane::Pane;
pub use pane_chrome::PaneChrome;
pub use pane_chrome::default_pane_chrome;
pub use pane_chrome::empty_pane_block;
pub use pane_id::FocusedPane;
pub use pane_id::FrameworkFocusId;
pub use pane_id::FrameworkOverlayId;
pub use pane_state::PaneFocusState;
pub use pane_state::PaneSelectionState;
pub use pane_state::RenderFocus;
pub use pane_state::scroll_indicator;
pub use pane_state::selection_state;
pub use pane_state::selection_state_for;
pub use pane_state::selection_style;
pub use pane_title::PaneTitleCount;
pub use pane_title::PaneTitleGroup;
pub use pane_title::pane_title;
pub use pane_title::prefixed_pane_title;
pub use popup::PopupAreas;
pub use popup::PopupFrame;
pub use popup::centered_rect;
pub use rules::PaneRule;
pub use rules::RuleTitle;
pub use rules::render_horizontal_rule;
pub use rules::render_rules;

// After
pub use pane::Mode;
pub use pane::Pane;
pub use pane::chrome::PaneChrome;
pub use pane::chrome::default_pane_chrome;
pub use pane::chrome::empty_pane_block;
pub use pane::id::FocusedPane;
pub use pane::id::FrameworkFocusId;
pub use pane::id::FrameworkOverlayId;
pub use pane::popup::PopupAreas;
pub use pane::popup::PopupFrame;
pub use pane::popup::centered_rect;
pub use pane::rules::PaneRule;
pub use pane::rules::RuleTitle;
pub use pane::rules::render_horizontal_rule;
pub use pane::rules::render_rules;
pub use pane::state::PaneFocusState;
pub use pane::state::PaneSelectionState;
pub use pane::state::RenderFocus;
pub use pane::state::scroll_indicator;
pub use pane::state::selection_state;
pub use pane::state::selection_state_for;
pub use pane::state::selection_style;
pub use pane::title::PaneTitleCount;
pub use pane::title::PaneTitleGroup;
pub use pane::title::pane_title;
pub use pane::title::prefixed_pane_title;
```

### Sequencing

Single commit; checkpoint with `cargo build -p tui_pane` + `cargo nextest run -p tui_pane` between steps.

1. Create `tui_pane/src/pane/` directory.
2. Move each file with `git mv` to preserve history:
   - `git mv src/pane.rs src/pane/mod.rs`
   - `git mv src/pane_chrome.rs src/pane/chrome.rs`
   - `git mv src/pane_id.rs src/pane/id.rs`
   - `git mv src/pane_state.rs src/pane/state.rs`
   - `git mv src/pane_title.rs src/pane/title.rs`
   - `git mv src/popup.rs src/pane/popup.rs`
   - `git mv src/rules.rs src/pane/rules.rs`
3. In `src/pane/mod.rs` (the moved former `pane.rs`), prepend `mod chrome; mod id; mod popup; mod rules; mod state; mod title;` above the existing `Pane<Ctx>` trait and `Mode<Ctx>` enum.
4. In `src/lib.rs`: remove the seven module declarations (`mod pane; mod pane_chrome; mod pane_id; mod pane_state; mod pane_title; mod popup; mod rules;`) and replace with a single `mod pane;`. Update the `pub use` block as shown above.
5. Fix the one intra-crate import: `src/pane/title.rs` (was `src/pane_title.rs`) had `use crate::pane_state;` — change to `use super::state;` and update references to `super::state::scroll_indicator`.
6. Rename `panes/` → `overlays/`:
   - `git mv src/panes src/overlays`
   - In `src/lib.rs`: rename `mod panes;` to `mod overlays;` and rewrite every `pub use panes::X;` line to `pub use overlays::X;`.
   - Grep code for `crate::panes::` and `use super::panes` (or sibling references inside the renamed dir) and rewrite each.
   - Grep doc comments and prose: `rg "panes::" tui_pane/src/ --include='*.rs'` — rustdoc intra-doc links (`[crate::panes::Foo]`) break silently at doc-build time, not compile time. Rewrite each hit.
7. `cargo build -p tui_pane` + `cargo nextest run -p tui_pane`; commit.

## Phase 2 — Create `layout/` submodule

Three geometry primitives at the root form a layout cluster: pane grid resolution, viewport scroll state, column fit-to-content. They become a `layout/` directory with `layout/mod.rs` carrying the pane-grid types.

### Target layout

```
tui_pane/src/
└── layout/
    ├── mod.rs             # PaneAxisSize, PaneSizeSpec, PanePlacement, PaneGridLayout, ResolvedPane*, constraints_for_sizes (current layout.rs contents)
    ├── column_widths.rs   # was column_widths.rs — ColumnSpec, ColumnWidths
    └── viewport.rs        # was viewport.rs — Viewport, ViewportOverflow, render_overflow_affordance
```

### Rationale

The three files are independent types but share a category: each computes positions or sizes for content placed inside a pane. Grouping under `layout/` cuts the root by two files and gives a single landing zone for future geometry primitives.

### lib.rs re-export paths

```rust
// Before
pub use column_widths::ColumnSpec;
pub use column_widths::ColumnWidths;
pub use viewport::Viewport;
pub use viewport::ViewportOverflow;
pub use viewport::render_overflow_affordance;

// After
pub use layout::column_widths::ColumnSpec;
pub use layout::column_widths::ColumnWidths;
pub use layout::viewport::Viewport;
pub use layout::viewport::ViewportOverflow;
pub use layout::viewport::render_overflow_affordance;
```

### Sequencing

Single commit; checkpoint between steps.

1. Create `tui_pane/src/layout/` directory.
2. Move files with `git mv` to preserve history:
   - `git mv src/layout.rs src/layout/mod.rs`
   - `git mv src/viewport.rs src/layout/viewport.rs`
   - `git mv src/column_widths.rs src/layout/column_widths.rs`
3. In `src/layout/mod.rs`, prepend `mod column_widths; mod viewport;`.
4. In `src/lib.rs`: remove the `mod column_widths;` and `mod viewport;` declarations (`mod layout;` stays). Update the `pub use` block as shown.
5. `cargo build -p tui_pane` + `cargo nextest run -p tui_pane`; commit.

## Settled layout after Phases 1–2

**Root files** — seven `.rs` siblings of `lib.rs`:
- `lib.rs`, `app_context.rs`, `constants.rs`, `util.rs` — crate-level concerns
- `activity.rs`, `running_tracker.rs`, `watched_file.rs` — each a standalone utility used by different subsystems with no shared type, so no clustering candidate

**Subdirectories** — `bar/`, `dispatch/`, `framework/`, `keymap/`, `layout/`, `overlays/`, `pane/`, `settings_store/`, `toasts/`. Each contains files grouped by a single concern.

The two single-domain files in `toasts/` (`action.rs` defining `ToastsAction`; `ids.rs` defining `ToastId` + `ToastTaskId`) stay where they are. Each is correctly named per `name-submodules-after-anchor-types.md` (anchor type and identity cohort respectively); the style rule penalizes incorrectly-named files, not small ones.

## Phase 3 — Extract tests from `keymap/builder/mod.rs`

`keymap/builder/mod.rs` is 1668 total lines: 470 production, 1198 inline test. The production code is cohesive (typestate transitions plus build finalization) and stays put. The test block — eleven integration-style cases that drive the full `Configuring → Registering → build()` cycle — moves out. This mirrors the `bar/tests.rs` pattern already used elsewhere in the crate.

### Target layout

```
keymap/builder/
├── mod.rs            # 470 prod lines (typestate, public surface, orchestration)
├── tests.rs          # ~1198 lines lifted from the #[cfg(test)] block
├── finalize.rs
├── overlay.rs
└── registration.rs
```

### Sequencing

Single commit; checkpoint with `cargo build -p tui_pane` + `cargo nextest run -p tui_pane` between steps.

1. Identify the `#[cfg(test)] mod tests { ... }` block at the bottom of `mod.rs`. Note any items it imports via `use super::*;` (this expands to the full production surface, so visibility is already wide enough).
2. Move the block body to a new `keymap/builder/tests.rs`. The file's top is `use super::*;` plus any standalone `use` lines previously inside the block.
3. Replace the inline block in `mod.rs` with `#[cfg(test)] mod tests;`.
4. Confirm no production item required `pub(super)` widening (the test block already saw `super::*` items).
5. `cargo build -p tui_pane` + `cargo nextest run -p tui_pane`; commit.

## Phase 4 — Split `toasts/stack.rs` by lifting `Toasts<Ctx>` into `toasts/mod.rs`

`Toasts<Ctx>` is the anchor type for the whole `toasts/` module — every other type in the subsystem (`Toast`, `ToastView`, `ToastCommand`, `ToastSettings`, …) exists to serve it. Its current home in `toasts/stack.rs` is the wrong placement under the anchor-type rule: the parent module name `toasts` matches the snake_case of `Toasts`, so the type belongs in `toasts/mod.rs`. `toasts/stack.rs` itself disappears; its 616 production lines redistribute across new toasts-level siblings, each holding one impl-block concern.

### Target layout

```
toasts/
├── mod.rs               # Toasts<Ctx> struct + ToastCommand + ToastSpec + new() + Default + sync_viewport_len() (private helper)
├── body.rs              # unchanged
├── commands.rs          # impl Toasts<Ctx> { push variants, dismiss, task mutators } — NEW
├── format.rs            # unchanged
├── item.rs              # unchanged
├── lifecycle.rs         # impl Toasts<Ctx> { prune, queries, tracked-item maintenance } — NEW
├── navigation.rs        # impl Toasts<Ctx> { focus + cursor stepping } — NEW
├── render.rs            # unchanged (Phase 5 splits this further)
├── settings.rs          # unchanged
├── slots.rs             # impl Toasts<Ctx> { mode, defaults, bar_slots, hits, set_hits, handle_key, handle_key_command, Hittable impl } — NEW
├── toast.rs             # unchanged
└── view.rs              # unchanged
```

Naming choice: `slots.rs` instead of `bar.rs` — the top-level `bar/` module already owns "bar". This submodule contributes status-bar slots and the input dispatch that drives them, so `slots` reads correctly without a name clash. If `slots.rs` grows substantially (mouse routing, focus handling, modal interactions), split key dispatch back out as `toasts/input.rs`.

### What goes where

Line ranges below reference the current `toasts/stack.rs`. When executing, locate by function/type name — the file has grown since the plan was written.

**`toasts/mod.rs`** — gains the manager and its constructor
- `struct ToastCommand<A>` (lines 38–43)
- `struct ToastSpec<Ctx>` (lines 45–53)
- `struct Toasts<Ctx>` definition (lines 56–62), `impl Default` (lines 64–66)
- `impl Toasts<Ctx> { pub fn new() }` constructor (lines 71–78)
- `impl Toasts<Ctx> { fn sync_viewport_len() }` private helper — kept here as a `pub(super)` shared helper because both `commands.rs` and `lifecycle.rs` invoke it (housing it in the parent avoids cross-sibling imports)
- Existing `mod` declarations + `pub use` block stays; new entries: `mod commands; mod dispatch; mod lifecycle; mod navigation; mod slots;`. The current `mod stack;` and `pub use stack::ToastCommand; pub use stack::Toasts;` lines are removed (types are now defined directly).

**`toasts/commands.rs`** (lines 81–222, 263–313, 412–427)
- `impl Toasts<Ctx> { push, push_styled, push_with_action, push_timed, push_timed_styled, push_task, push_persistent, push_persistent_styled }`
- `impl Toasts<Ctx> { start_task, finish_task, reactivate_task, update_task_body, set_tracked_items }`
- `impl Toasts<Ctx> { dismiss, dismiss_focused }`
- `impl Toasts<Ctx> { mark_item_completed, mark_tracked_item_completed }`
- Private helper `push_entry()` (lines 557–575)

**`toasts/lifecycle.rs`** (lines 227–260, 317–409, 430–453)
- `impl Toasts<Ctx> { has_active, active, active_now, active_views, focused_toast_id }`
- `impl Toasts<Ctx> { is_alive, is_task_finished, tracked_item_count }`
- `impl Toasts<Ctx> { complete_missing_items, add_new_tracked_items, restart_tracked_item }`
- `impl Toasts<Ctx> { prune, prune_tracked_items }`
- Private helpers `toast_for_task()`, `toast_for_task_mut()`
- Calls `Self::sync_viewport_len()` defined in `mod.rs`

**`toasts/navigation.rs`** (lines 226, 456–501)
- `impl Toasts<Ctx> { focused_id, reset_to_first, reset_to_last }`
- `impl Toasts<Ctx> { on_navigation }` (consumes a `CycleDirection`)
- `impl Toasts<Ctx> { try_consume_cycle_step }`

**`toasts/slots.rs`** (lines 504–555, 600–614)
- `impl Toasts<Ctx> { mode, defaults }`
- `impl Toasts<Ctx> { bar_slots }`
- `impl Toasts<Ctx> { hits, set_hits }`
- `impl Toasts<Ctx> { handle_key_command, handle_key }` — input dispatch lives here alongside the bar surface it drives
- `impl Hittable for Toasts<Ctx>`

### Visibility changes required

Rust gotcha: an inherent method's privacy is scoped to the module where the `impl` block lives, not where the type is defined. Two `impl Toasts<Ctx>` blocks in sibling files (`commands.rs` and `lifecycle.rs`) cannot see each other's private methods just by virtue of being on the same type.

This restructure dodges that issue by design:

- **`sync_viewport_len` lives in `toasts/mod.rs`.** Both `commands.rs` and `lifecycle.rs` invoke it via `self.sync_viewport_len()`. Child modules can call private items of their parent module, so this works without `pub(super)`.
- **`push_entry`** stays in `commands.rs`. Verified by grep: its three callers (the various `push_*` methods) all land in `commands.rs`. No cross-sibling call; stays private.
- **`toast_for_task`, `toast_for_task_mut`** stay in `lifecycle.rs`. All callers (task-status query and mutation methods) also land in `lifecycle.rs`. No cross-sibling call; stay private.

If a future caller of one of these helpers lands in a different sibling, that helper widens to `pub(super)` at that point. For the initial split, no widening is needed.

No public surface changes. `tui_pane::Toasts` and `tui_pane::ToastCommand` continue to work through the crate-root re-exports; the underlying paths simplify from `tui_pane::toasts::stack::Toasts` to `tui_pane::toasts::Toasts`.

### Sequencing

Single commit; checkpoint with `cargo build -p tui_pane` + `cargo nextest run -p tui_pane` between steps.

1. Open `toasts/stack.rs`. Cut the struct definitions (`ToastCommand<A>`, `ToastSpec<Ctx>`, `Toasts<Ctx>`), the `impl Default for Toasts<Ctx>`, the `new()` constructor, and the `sync_viewport_len` helper. Paste them into `toasts/mod.rs` (after the existing `mod` declarations).
2. In `toasts/mod.rs`: remove `mod stack;` and the `pub use stack::*` re-exports for `Toasts` and `ToastCommand` — those types are defined inline now.
3. Create `toasts/slots.rs` with the bar-slot methods, `Hittable` impl, and the two key-dispatch methods (`handle_key`, `handle_key_command`). In `mod.rs`, add `mod slots;`.
4. Create `toasts/navigation.rs` with the cursor-stepping methods. In `mod.rs`, add `mod navigation;`.
5. Create `toasts/commands.rs` with the push/dismiss/task-mutator methods plus the `push_entry` helper. In `mod.rs`, add `mod commands;`.
6. Create `toasts/lifecycle.rs` with the prune/query/tracked-item methods plus `toast_for_task` helpers. In `mod.rs`, add `mod lifecycle;`.
7. Relocate the `#[cfg(test)] mod tests { ... }` block at the bottom of `stack.rs` (~241 lines covering 11 cases that exercise the full `Toasts<Ctx>` manager) into `toasts/tests.rs`. The block uses `use super::*;` and tests the manager as a whole, so a single co-located test file matches the existing `bar/tests.rs` precedent better than scattering tests across the four new siblings. In `mod.rs`, add `#[cfg(test)] mod tests;`.
8. Delete the now-empty `toasts/stack.rs`. The `mod stack;` declaration was already removed in step 2.
9. `cargo build -p tui_pane` + `cargo nextest run -p tui_pane`; commit.

## Phase 5 — Split `toasts/render.rs`

`toasts/render.rs` is 587 production lines containing the toast rendering pipeline: a public entry point, two-direction layout allocation, per-card drawing with borders / styles / animation phases, and a cluster of text formatting helpers (fade, truncate, elapsed). Pass 2 call-site review confirmed render is independent of `stack.rs` (it works on `&[ToastView]` slices), so this split is fully self-contained.

### Target layout

```
toasts/render/
├── mod.rs            # render_toasts entry, ToastRenderResult, ToastsRenderCtx, Renderable impl
├── layout.rs         # StackLayout, render_top_down, render_bottom_up, allocate_toast_heights
├── card.rs           # render_toast, render_toast_body, body_lines_plain, body_lines_tracked, tracked_item_line
└── format.rs         # fade_to_style, fade_to_color, truncate, truncate_with_ellipsis, format_elapsed
```

### What goes where

Line ranges below reference the current `toasts/render.rs`.

**`render/mod.rs`** — retained production code
- Color constants (27–32): `ACCENT_COLOR`, `ACTIVE_BORDER_COLOR`, etc.
- `struct ToastRenderResult` (35–38)
- `pub fn render_toasts` (41–82)
- `struct ToastsRenderCtx<'a>` (90–102)
- `impl Renderable<ToastsRenderCtx<'_>> for super::Toasts<Ctx>` (104–118)

**`render/layout.rs`** (lines 121–259)
- `struct StackLayout`
- `fn render_top_down`, `fn render_bottom_up`
- `fn allocate_toast_heights`

**`render/card.rs`** (lines 261–532)
- `fn render_toast`, `fn render_toast_body`
- `fn body_lines_plain`, `fn body_lines_tracked`, `fn tracked_item_line`

**`render/format.rs`** (lines 534–586)
- `fn fade_to_style`, `fn fade_to_color`
- `fn truncate`, `fn truncate_with_ellipsis`
- `fn format_elapsed`

### Visibility changes required

All extracted helpers are currently private; widen to `pub(super)`. The public surface (`render_toasts`, `ToastRenderResult`, `ToastsRenderCtx`) stays in `mod.rs` and remains `pub`.

### Sequencing

Single commit; checkpoint between steps. Leaves first.

1. Create `toasts/render/` directory; move the current `render.rs` content into `render/mod.rs` unchanged. Sanity check.
2. Extract `format.rs` (pure functions, zero inter-module deps).
3. Extract `layout.rs` (depends only on ratatui + `ToastView` / `ToastHitbox`).
4. Extract `card.rs` (depends on `format.rs` helpers).
5. `mod.rs` keeps the entry point and re-exports nothing additional — submodule items stay `pub(super)`.
6. `cargo build -p tui_pane` + `cargo nextest run -p tui_pane`; commit.

## Deferred items

**`panes/settings.rs` test extraction.** The file is 986 total / 461 production / 525 test (53% test). Test extraction looks attractive on the ratio, but the `#[cfg(test)] impl SettingsPane { fn for_test_editing(...) }` block builds the struct via a private-field literal, so that block cannot move without widening private fields to `pub(super)`. Only the `mod tests { ... }` body (~229 lines) can extract cleanly, which is a small reclaim against 461 production lines that already sit below the 500-line split threshold. Severity: minor. Revisit if `settings.rs` grows past 600 lines or accumulates a second test block.

**`settings_store::store` imports `crate::toasts::ToastSettings`.** A generic persistence layer pulling a UI-feature type is a backward dependency. Options: (a) move `ToastSettings` to a shared module the toasts layer also imports, (b) make `SettingsStore` generic over the settings types it serializes. Severity: important. Cost: ~1 day. Not blocking; the import is one line and isolated.

**`keymap::Keymap<Ctx>` stores `ScopeMap<SettingsPaneAction>` and `ScopeMap<KeymapPaneAction>`.** Generic keymap container knows about two framework overlay action types by name. After Phase 1 the action types live in `overlays/` (formerly `panes/`); the issue itself is unchanged. Move overlay scope maps to `Framework<Ctx>`; `Keymap` keeps only the navigation scope plus the type-erased map for app-pane scopes. Severity: important. Cost: ~1 day; touches `keymap/mod.rs`, `keymap/builder/mod.rs`, and the framework wiring.
